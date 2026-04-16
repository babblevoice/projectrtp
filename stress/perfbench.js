/*
 * perfbench — measure projectrtp scheduler / pipeline overhead under load.
 *
 * Opens N channels with echo enabled, sends RTP into each at ptime, listens
 * for the echoed-back packets, and reports drop rate + latency percentiles +
 * CPU usage. The same harness runs against both the C++ and Rust builds —
 * swap `build/Release/projectrtp.node` between runs and compare.
 *
 * Usage:
 *   CHANNELS=500 DURATION_MS=10000 node stress/perfbench.js
 *
 * Typical comparison:
 *   # C++ build
 *   npm run build && node stress/perfbench.js
 *   # Rust build
 *   (cd rust && cargo build --release) && \
 *     ln -f rust/target/release/libprojectrtp.so build/Release/projectrtp.node && \
 *     node stress/perfbench.js
 */

const dgram = require( "dgram" )
const os = require( "os" )
const { performance } = require( "perf_hooks" )
const { projectrtp } = require( "../index.js" )

const CHANNELS = Number( process.env.CHANNELS || 500 )
const DURATION_MS = Number( process.env.DURATION_MS || 10_000 )
const PTIME_MS = Number( process.env.PTIME_MS || 20 )
const PAYLOAD_BYTES = 160 // PCMU @ 8kHz, 20ms
const PT = 0 // PCMU

function buildpk( sn, ssrc ) {
  const pkt = Buffer.alloc( 12 + PAYLOAD_BYTES )
  pkt[ 0 ] = 0x80
  pkt[ 1 ] = PT
  pkt.writeUInt16BE( sn & 0xffff, 2 )
  pkt.writeUInt32BE( ( sn * PAYLOAD_BYTES ) >>> 0, 4 )
  pkt.writeUInt32BE( ssrc >>> 0, 8 )
  pkt.fill( 0xff, 12 ) // PCMU silence
  return pkt
}

function percentile( sorted, q ) {
  if ( sorted.length === 0 ) return 0
  const idx = Math.min( sorted.length - 1, Math.floor( sorted.length * q ) )
  return sorted[ idx ]
}

async function main() {
  projectrtp.run()

  const channels = []
  const endpoints = []
  const sendTimes = new Map() // key: `${chanIdx}:${sn}` → sendTime (ms)

  let totalSent = 0
  let totalReceived = 0
  const latencies = []

  console.log( `perfbench: opening ${CHANNELS} channels...` )

  for ( let i = 0; i < CHANNELS; i++ ) {
    const endpoint = dgram.createSocket( "udp4" )
    endpoint.bind()
    await new Promise( ( r ) => endpoint.on( "listening", r ) )

    const port = endpoint.address().port
    const chan = await projectrtp.openchannel( {
      forcelocal: true,
      remote: { address: "127.0.0.1", port, codec: PT },
    }, () => {} )
    chan.echo()

    const chanIdx = i
    endpoint.on( "message", ( msg ) => {
      if ( msg.length < 12 ) return
      const sn = msg.readUInt16BE( 2 )
      const key = `${chanIdx}:${sn}`
      const t0 = sendTimes.get( key )
      if ( t0 !== undefined ) {
        latencies.push( performance.now() - t0 )
        sendTimes.delete( key )
        totalReceived++
      }
    } )

    endpoints.push( endpoint )
    channels.push( chan )
  }

  // Give channels a beat to settle before we start timing.
  await new Promise( ( r ) => setTimeout( r, 200 ) )

  console.log( `perfbench: sending for ${DURATION_MS}ms (ptime=${PTIME_MS}ms)...` )

  const cpuStart = process.cpuUsage()
  const wallStart = performance.now()
  let sn = 0

  const sendTimer = setInterval( () => {
    const now = performance.now()
    for ( let i = 0; i < CHANNELS; i++ ) {
      const pkt = buildpk( sn, 0x10000000 + i )
      const key = `${i}:${sn & 0xffff}`
      sendTimes.set( key, now )
      totalSent++
      endpoints[ i ].send( pkt, channels[ i ].local.port, "127.0.0.1" )
    }
    sn++
    if ( now - wallStart >= DURATION_MS ) {
      clearInterval( sendTimer )
      finalize()
    }
  }, PTIME_MS )

  async function finalize() {
    // Let in-flight echoes arrive.
    await new Promise( ( r ) => setTimeout( r, 500 ) )

    const cpu = process.cpuUsage( cpuStart )
    const wallMs = performance.now() - wallStart

    for ( const ch of channels ) ch.close()
    for ( const ep of endpoints ) ep.close()

    latencies.sort( ( a, b ) => a - b )
    const dropPct = totalSent ? ( ( totalSent - totalReceived ) / totalSent * 100 ) : 0
    const userMs = cpu.user / 1000
    const sysMs = cpu.system / 1000
    const cpuMs = userMs + sysMs
    const coreCount = os.cpus().length

    console.log()
    console.log( `Config:` )
    console.log( `  channels:     ${CHANNELS}` )
    console.log( `  duration:     ${DURATION_MS}ms (wall ${wallMs.toFixed( 0 )}ms)` )
    console.log( `  ptime:        ${PTIME_MS}ms` )
    console.log( `  cores:        ${coreCount}` )
    console.log()
    console.log( `Throughput:` )
    console.log( `  sent:         ${totalSent}` )
    console.log( `  received:     ${totalReceived}` )
    console.log( `  drop rate:    ${dropPct.toFixed( 3 )}%` )
    console.log()
    console.log( `Echo latency (ms):` )
    console.log( `  p50:          ${percentile( latencies, 0.50 ).toFixed( 2 )}` )
    console.log( `  p95:          ${percentile( latencies, 0.95 ).toFixed( 2 )}` )
    console.log( `  p99:          ${percentile( latencies, 0.99 ).toFixed( 2 )}` )
    console.log( `  max:          ${percentile( latencies, 1 ).toFixed( 2 )}` )
    console.log()
    console.log( `CPU:` )
    console.log( `  user:         ${userMs.toFixed( 0 )}ms` )
    console.log( `  sys:          ${sysMs.toFixed( 0 )}ms` )
    console.log( `  cpu / wall:   ${( cpuMs / wallMs * 100 ).toFixed( 1 )}% (of 1 core)` )
    console.log( `  cpu / all:    ${( cpuMs / wallMs / coreCount * 100 ).toFixed( 1 )}% (of ${coreCount} cores)` )

    setTimeout( () => process.exit( 0 ), 100 )
  }
}

main().catch( ( e ) => { console.error( e ); process.exit( 1 ) } )
