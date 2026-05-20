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
const MODE = process.env.MODE || "echo" // "echo" | "mix2" | "idle"
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

/* Memory snapshot — node's process.memoryUsage(). `rss` is resident
 * set size (bytes), the useful proxy for "how much RAM is Rust +
 * the Node shim using right now". We diff before/after channel open
 * to report per-channel overhead. */
function memSnapshot() {
  const m = process.memoryUsage()
  return { rss: m.rss, heap: m.heapUsed }
}

async function main() {
  projectrtp.run()

  const baselineMem = memSnapshot()

  const channels = []
  const endpoints = []
  const sendTimes = new Map() // key: `${chanIdx}:${sn}` → sendTime (ms)

  let totalSent = 0
  let totalReceived = 0
  const latencies = []

  console.log( `perfbench: mode=${MODE}  opening ${CHANNELS} channels...` )

  // In "mix2" mode we open channels in pairs and mix them. Each pair
  // shares ONE endpoint (even-indexed channel sends; odd-indexed
  // receives) — simulates the typical "A calls B" bridging scenario.
  // Latency is measured round-trip: A's endpoint sends, the mix
  // forwards to B's endpoint, B echoes back, mix forwards to A.
  for ( let i = 0; i < CHANNELS; i++ ) {
    const endpoint = dgram.createSocket( "udp4" )
    endpoint.bind()
    await new Promise( ( r ) => endpoint.on( "listening", r ) )

    const port = endpoint.address().port
    const chan = await projectrtp.openchannel( {
      forcelocal: true,
      remote: { address: "127.0.0.1", port, codec: PT },
    }, () => {} )

    if ( MODE === "echo" ) chan.echo()

    const chanIdx = i
    endpoint.on( "message", ( msg ) => {
      if ( msg.length < 12 ) return
      // In mix2 mode the odd-side endpoints echo back so A hears its
      // own audio — same round-trip shape as echo mode for latency.
      if ( MODE === "mix2" && ( chanIdx % 2 === 1 ) ) {
        endpoint.send( msg, chan.local.port, "127.0.0.1" )
        return
      }
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

  if ( MODE === "mix2" ) {
    for ( let i = 0; i < CHANNELS; i += 2 ) {
      if ( i + 1 < CHANNELS ) channels[ i ].mix( channels[ i + 1 ] )
    }
  }

  const afterOpenMem = memSnapshot()
  const perChannelMemKb = ( ( afterOpenMem.rss - baselineMem.rss ) / CHANNELS ) / 1024

  // Give channels a beat to settle before we start timing.
  await new Promise( ( r ) => setTimeout( r, 200 ) )

  console.log( `perfbench: sending for ${DURATION_MS}ms (ptime=${PTIME_MS}ms)...` )

  const cpuStart = process.cpuUsage()
  const wallStart = performance.now()
  let sn = 0

  // In mix2 mode only the even-indexed endpoints originate traffic;
  // the odd-indexed ones just echo, so sending them packets would be
  // redundant and would also mess with the latency measurement. In
  // echo / idle modes every channel originates.
  const sendEvery = ( MODE === "mix2" ) ? 2 : 1

  const sendTimer = setInterval( () => {
    const now = performance.now()
    if ( MODE !== "idle" ) {
      for ( let i = 0; i < CHANNELS; i += sendEvery ) {
        const pkt = buildpk( sn, 0x10000000 + i )
        const key = `${i}:${sn & 0xffff}`
        sendTimes.set( key, now )
        totalSent++
        endpoints[ i ].send( pkt, channels[ i ].local.port, "127.0.0.1" )
      }
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
    const peakMem = memSnapshot()

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
    console.log( `  mode:         ${MODE}` )
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
    console.log()
    console.log( `Memory:` )
    console.log( `  baseline rss: ${( baselineMem.rss / 1024 / 1024 ).toFixed( 1 )} MiB` )
    console.log( `  after-open:   ${( afterOpenMem.rss / 1024 / 1024 ).toFixed( 1 )} MiB` )
    console.log( `  peak:         ${( peakMem.rss / 1024 / 1024 ).toFixed( 1 )} MiB` )
    console.log( `  per channel:  ${perChannelMemKb.toFixed( 1 )} KiB` )

    setTimeout( () => process.exit( 0 ), 100 )
  }
}

main().catch( ( e ) => { console.error( e ); process.exit( 1 ) } )
