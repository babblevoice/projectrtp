/*
  Packet capture (MONITORING-20).

  These tests decode what we wrote with this repo's own pcap reader
  (test/interface/pcap.js) rather than trusting the writer, which is also the
  point of writing LINKTYPE_ETHERNET: a capture taken from a live call can be
  replayed straight back into the DTMF and codec tests.

  projectrtp.run() comes from the global before hook in projectrtpserver.js.
*/
const expect = require( "chai" ).expect
const dgram = require( "dgram" )
const fs = require( "fs" )
const projectrtp = require( "../../index.js" ).projectrtp
const readpcap = require( "./pcap.js" ).readpcap

const tmpfile = ( name ) => `/tmp/projectrtp-pcap-${ name }-${ process.pid }.pcap`

/**
 * Shannon entropy in bits per byte. Cipher output sits at ~8.0; G.711 audio
 * is far lower. Used to prove a capture is cleartext.
 * @param { Buffer } buf
 * @returns { number }
 */
function entropy( buf ) {
  const counts = new Array( 256 ).fill( 0 )
  for( const b of buf ) counts[ b ]++

  let h = 0
  for( const c of counts ) {
    if( 0 === c ) continue
    const p = c / buf.length
    h -= p * Math.log2( p )
  }
  return h
}

/**
 * @param { number } sn
 * @param { number } dstport
 * @param { dgram.Socket } server
 */
function sendpk( sn, dstport, server ) {
  return setTimeout( () => {
    const subheader = Buffer.alloc( 10 )
    subheader.writeUInt16BE( ( sn + 100 ) % ( 2 ** 16 ) )
    subheader.writeUInt32BE( sn * 160, 2 )
    subheader.writeUInt32BE( 25, 6 )

    server.send( Buffer.concat( [
      Buffer.from( [ 0x80, 0x00 ] ),
      subheader,
      Buffer.alloc( 160 ).fill( sn & 0xff ) ] ), dstport, "127.0.0.1" )
  }, sn * 20 )
}

/**
 * Drive a channel with 50 inbound packets and echo, capturing to file.
 * @param { string } file
 * @param { object } pcapoptions
 * @param { boolean } finishexplicitly
 * @returns { Promise< object > }
 */
async function capture( file, pcapoptions = {}, finishexplicitly = true ) {

  const server = dgram.createSocket( "udp4" )
  server.on( "message", () => {} )
  await new Promise( ( r ) => { server.bind( r ) } )

  const events = []
  const channel = await projectrtp.openchannel( {
    "remote": { "address": "127.0.0.1", "port": server.address().port, "codec": 0 }
  }, ( d ) => { events.push( d ) } )

  const armed = channel.pcap( { "file": file, ...pcapoptions } )
  channel.echo()

  for( let i = 0; 50 > i; i++ ) sendpk( i, channel.local.port, server )
  await new Promise( ( r ) => { setTimeout( r, 1300 ) } )

  if( finishexplicitly ) channel.pcap( { "finish": true } )
  await new Promise( ( r ) => { setTimeout( r, 200 ) } )

  /* a capture that stopped on a limit must say so when it stops, not when
     the channel later closes */
  const beforeclose = events.some( ( e ) => "pcap" === e.action )

  channel.close()
  await new Promise( ( r ) => { setTimeout( r, 300 ) } )
  server.close()

  return { armed, beforeclose, "event": events.find( ( e ) => "pcap" === e.action ) }
}

describe( "pcap", function() {

  it( "captures both directions and decodes cleanly", async function() {

    this.timeout( 6000 )
    this.slow( 3000 )

    const file = tmpfile( "both" )
    const { armed, event } = await capture( file )

    expect( armed ).to.be.true
    expect( event.reason ).to.equal( "requested" )
    expect( event.packets ).to.equal( 100 )
    expect( event.skipped ).to.equal( 0 )

    const frames = await readpcap( file )
    expect( frames ).to.be.an( "array" )
    expect( frames.length ).to.equal( 100 )

    /* every frame must decode as IPv4/UDP - a link type or header mistake
       shows up here as undefined rather than as a wrong value */
    frames.forEach( ( f ) => {
      expect( f.ethertype ).to.equal( "0800" )
      expect( f.ipv4.protocol ).to.equal( 17 )
      expect( f.ipv4.udp.data.length ).to.equal( 172 )
    } )

    /* both directions present */
    const flows = new Set( frames.map( ( f ) => `${ f.ipv4.udp.srcport }->${ f.ipv4.udp.dstport }` ) )
    expect( flows.size ).to.equal( 2 )

    /* and the payload really is the rtp we sent */
    const rtp = frames[ 0 ].ipv4.udp.data
    expect( rtp[ 0 ] >> 6 ).to.equal( 2, "rtp version" )
    expect( rtp[ 1 ] & 0x7f ).to.equal( 0, "pcmu" )

    await fs.promises.unlink( file ).catch( () => {} )
  } )

  it( "names our end with the advertised address, not 0.0.0.0", async function() {

    this.timeout( 6000 )
    this.slow( 3000 )

    /* the rtp sockets bind 0.0.0.0, so without the substitution every capture
       names one end 0.0.0.0 and reads as broken */
    const file = tmpfile( "address" )
    await capture( file, { "localaddress": "10.20.30.40" } )

    const frames = await readpcap( file )
    const addresses = new Set()
    frames.forEach( ( f ) => { addresses.add( f.ipv4.src ); addresses.add( f.ipv4.dst ) } )

    expect( addresses.has( "10.20.30.40" ) ).to.be.true
    expect( addresses.has( "0.0.0.0" ) ).to.be.false

    await fs.promises.unlink( file ).catch( () => {} )
  } )

  it( "stops at maxsize and still leaves a readable file", async function() {

    this.timeout( 6000 )
    this.slow( 3000 )

    const file = tmpfile( "maxsize" )
    const { event, beforeclose } = await capture( file, { "maxsize": 4096 }, false )

    /* reported when the limit is hit, so babble-rtp can upload it then
       rather than when the call ends */
    expect( beforeclose ).to.be.true
    expect( event.reason ).to.equal( "maxsize" )
    expect( event.filesize ).to.be.at.most( 4096 )
    expect( event.packets ).to.be.above( 0 )

    const frames = await readpcap( file )
    expect( frames.length ).to.equal( event.packets )

    await fs.promises.unlink( file ).catch( () => {} )
  } )

  it( "stops at maxduration and still leaves a readable file", async function() {

    this.timeout( 6000 )
    this.slow( 3000 )

    /* a full run is 100 packets over about a second */
    const file = tmpfile( "maxduration" )
    const { event, beforeclose } = await capture( file, { "maxduration": 400 }, false )

    expect( beforeclose ).to.be.true
    expect( event.reason ).to.equal( "maxduration" )
    expect( event.packets ).to.be.above( 0 )
    expect( event.packets ).to.be.below( 100 )

    const frames = await readpcap( file )
    expect( frames.length ).to.equal( event.packets )

    await fs.promises.unlink( file ).catch( () => {} )
  } )

  it( "finishes the capture when the channel closes", async function() {

    this.timeout( 6000 )
    this.slow( 3000 )

    /* a caller who simply hangs up must still leave a usable file */
    const file = tmpfile( "close" )
    const { event, beforeclose } = await capture( file, {}, false )

    /* no limit, so nothing to report until the channel closes */
    expect( beforeclose ).to.be.false
    expect( event.reason ).to.equal( "channelclosed" )
    expect( event.packets ).to.equal( 100 )

    const frames = await readpcap( file )
    expect( frames.length ).to.equal( 100 )

    await fs.promises.unlink( file ).catch( () => {} )
  } )

  it( "reports a file it cannot open rather than failing silently", async function() {

    this.timeout( 6000 )
    this.slow( 3000 )

    const { armed, event } = await capture( "/proc/nope/cannot.pcap" )

    expect( armed ).to.be.true
    expect( event.reason ).to.have.string( "open-failed" )
    expect( event.packets ).to.equal( 0 )
  } )

  it( "refuses to arm without a file", async function() {

    this.timeout( 6000 )
    this.slow( 3000 )

    const channel = await projectrtp.openchannel( {}, () => {} )
    expect( channel.pcap( {} ) ).to.be.false
    channel.close()
  } )

  it( "captures a bridged (mixed) call, both legs", async function() {

    this.timeout( 8000 )
    this.slow( 4000 )

    /* Real calls are bridged, not echoed - the mixer has its own taps and is
       the path that actually matters in production. */
    const filea = tmpfile( "mixa" )
    const fileb = tmpfile( "mixb" )

    const endpointa = dgram.createSocket( "udp4" )
    const endpointb = dgram.createSocket( "udp4" )
    endpointa.on( "message", () => {} )
    endpointb.on( "message", () => {} )

    await new Promise( ( r ) => { endpointa.bind( r ) } )
    await new Promise( ( r ) => { endpointb.bind( r ) } )

    const events = []
    const channela = await projectrtp.openchannel( {
      "remote": { "address": "127.0.0.1", "port": endpointa.address().port, "codec": 0 }
    }, ( d ) => { events.push( d ) } )

    const channelb = await projectrtp.openchannel( {
      "remote": { "address": "127.0.0.1", "port": endpointb.address().port, "codec": 0 }
    }, ( d ) => { events.push( d ) } )

    expect( channela.mix( channelb ) ).to.be.true

    /* capture both legs of the bridge at once */
    expect( channela.pcap( { "file": filea } ) ).to.be.true
    expect( channelb.pcap( { "file": fileb } ) ).to.be.true

    for( let i = 0; 50 > i; i++ ) sendpk( i, channela.local.port, endpointa )
    await new Promise( ( r ) => { setTimeout( r, 1400 ) } )

    channela.close()
    channelb.close()
    await new Promise( ( r ) => { setTimeout( r, 400 ) } )
    endpointa.close()
    endpointb.close()

    const pcaps = events.filter( ( e ) => "pcap" === e.action )
    expect( pcaps.length ).to.equal( 2 )
    pcaps.forEach( ( e ) => { expect( e.packets ).to.be.above( 0 ) } )

    /* the leg being fed must see inbound; the far leg must see the relayed
       audio the mixer sent it, so both files carry traffic */
    const framesa = await readpcap( filea )
    const framesb = await readpcap( fileb )
    expect( framesa.length ).to.be.above( 40 )
    expect( framesb.length ).to.be.above( 40 )

    framesa.forEach( ( f ) => { expect( f.ipv4.udp.data.length ).to.equal( 172 ) } )
    framesb.forEach( ( f ) => { expect( f.ipv4.udp.data.length ).to.equal( 172 ) } )

    await fs.promises.unlink( filea ).catch( () => {} )
    await fs.promises.unlink( fileb ).catch( () => {} )
  } )

  it( "captures a dtls-srtp leg as cleartext", async function() {

    this.timeout( 10000 )
    this.slow( 6000 )

    /* The capture taps sit inside the crypto boundary - inbound after
       decryption, outbound before encryption - so an encrypted call must
       capture as plain rtp with no key material to hand over. Proven by
       entropy: cipher output is ~8 bits/byte, g711 audio is far below that. */
    const file = tmpfile( "dtls" )
    projectrtp.tone.generate( "400+450*0.5/0/400+450*0.5/0:400/200/400/2000", "/tmp/pcapringing.wav" )

    const keepalive = setInterval( () => {}, 50 )
    const events = []
    let done
    const finished = new Promise( ( r ) => { done = r } )

    const channela = await projectrtp.openchannel( {}, ( d ) => {
      if( "close" === d.action ) channelb.close()
    } )
    const channelb = await projectrtp.openchannel( {}, ( d ) => {
      events.push( d )
      if( "close" === d.action ) done()
    } )

    expect( channela.remote( {
      "address": "127.0.0.1", "port": channelb.local.port, "codec": 0,
      "dtls": { "fingerprint": { "hash": channelb.local.dtls.fingerprint }, "mode": "active" }
    } ) ).to.be.true

    expect( channelb.remote( {
      "address": "127.0.0.1", "port": channela.local.port, "codec": 0,
      "dtls": { "fingerprint": { "hash": channela.local.dtls.fingerprint }, "mode": "passive" }
    } ) ).to.be.true

    expect( channelb.pcap( { "file": file } ) ).to.be.true

    channela.play( { "loop": true, "files": [ { "wav": "/tmp/pcapringing.wav" } ] } )
    channelb.echo()

    await new Promise( ( r ) => { setTimeout( r, 2500 ) } )
    channela.close()
    await Promise.race( [ finished, new Promise( ( r ) => setTimeout( r, 4000 ) ) ] )
    await new Promise( ( r ) => { setTimeout( r, 300 ) } )
    clearInterval( keepalive )

    /* media actually flowed, which means the handshake succeeded - projectrtp
       withholds media rather than downgrading to plaintext, so a count here
       proves we really were encrypted on the wire */
    const closeevent = events.find( ( e ) => "close" === e.action )
    expect( closeevent.stats.in.count ).to.be.above( 70 )
    expect( closeevent.stats.in.skip ).to.equal( 0 )

    const frames = await readpcap( file )
    expect( frames.length ).to.be.above( 70 )

    /* no srtp auth tag - the wire form is 10 bytes longer */
    frames.forEach( ( f ) => { expect( f.ipv4.udp.data.length ).to.equal( 172 ) } )

    const payload = Buffer.concat( frames.map( ( f ) => f.ipv4.udp.data.subarray( 12 ) ) )
    expect( entropy( payload ) ).to.be.below( 7,
      "captured payload looks like ciphertext - the tap is on the wrong side of srtp" )

    await fs.promises.unlink( file ).catch( () => {} )
    await fs.promises.unlink( "/tmp/pcapringing.wav" ).catch( () => {} )
  } )
} )
