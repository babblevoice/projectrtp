/*
  Packet capture (MONITORING-20).

  The capture is taken at the socket boundary: every datagram a channel's
  sockets receive, as it arrives, and every datagram they send, as it leaves
  (after SRTP). So these tests look for what was on the wire - RTP, RTCP,
  DTLS - not for what the call sounded like.

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

/** @import { pcappacket } from "./pcap.js" */

const tmpfile = ( name ) => `/tmp/projectrtp-pcap-${ name }-${ process.pid }.pcap`

/**
 * Shannon entropy in bits per byte. Cipher output sits at ~8.0; G.711 audio
 * is far lower. Used to prove a capture holds what was on the wire.
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
 * Is this captured UDP payload RTCP? RFC 5761: RTCP packet types 192-223 sit
 * where an RTP payload type would be 64-95, which RTP never uses.
 * @param { Buffer } d
 * @returns { boolean }
 */
const isrtcp = ( d ) => 2 === d[ 0 ] >> 6 && 64 <= ( d[ 1 ] & 0x7f ) && 95 >= ( d[ 1 ] & 0x7f )

/**
 * @param { Buffer } d
 * @returns { boolean }
 */
const isrtp = ( d ) => 2 === d[ 0 ] >> 6 && !isrtcp( d )

/**
 * The frames of a capture that carry RTP.
 * @param { Array< pcappacket > } frames
 * @returns { Array< pcappacket > }
 */
const rtpframes = ( frames ) => frames.filter( ( f ) => isrtp( f.ipv4.udp.data ) )

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
 * What capture() saw.
 * @typedef { object } captureresult
 * @property { boolean } armed - what channel.pcap() returned when the capture was asked for
 * @property { boolean } beforeclose - whether the pcap event came before the channel was closed
 * @property { Object< string, any > } event - the channel's pcap event
 */

/**
 * Drive a channel with 50 inbound packets and echo, capturing to file.
 * @param { string } file
 * @param { object } pcapoptions
 * @param { boolean } finishexplicitly
 * @returns { Promise< captureresult > }
 */
async function capture( file, pcapoptions = {}, finishexplicitly = true ) {

  const server = dgram.createSocket( "udp4" )
  server.on( "message", () => {} )
  await /** @type { Promise< void > } */ ( new Promise( ( r ) => { server.bind( r ) } ) )

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
    expect( event.error ).to.be.undefined
    expect( event.skipped ).to.equal( 0 )
    expect( event.dropped ).to.equal( 0 )

    const allframes = await readpcap( file )
    expect( allframes ).to.be.an( "array" )
    expect( allframes.length ).to.equal( event.packets )

    /* every frame must decode as IPv4/UDP - a link type or header mistake
       shows up here as undefined rather than as a wrong value */
    allframes.forEach( ( f ) => {
      expect( f.ethertype ).to.equal( "0800" )
      expect( f.ipv4.protocol ).to.equal( 17 )
    } )

    /* 50 in and 50 echoed out; anything else is RTCP */
    const frames = rtpframes( allframes )
    expect( frames.length ).to.equal( 100 )
    frames.forEach( ( f ) => { expect( f.ipv4.udp.data.length ).to.equal( 172 ) } )

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

    const frames = await readpcap( file )
    expect( frames.length ).to.equal( event.packets )
    expect( rtpframes( frames ).length ).to.equal( 100 )

    /* the capture is closed after the RTCP BYE goes, so the file shows the
       stream being ended */
    const rtcp = frames.filter( ( f ) => isrtcp( f.ipv4.udp.data ) )
    const bye = rtcp.some( ( f ) => {
      const d = f.ipv4.udp.data
      for( let off = 0; off + 4 <= d.length; off += ( d.readUInt16BE( off + 2 ) + 1 ) * 4 ) {
        if( 203 === d[ off + 1 ] ) return true
      }
      return false
    } )
    expect( bye, "an RTCP BYE in the capture" ).to.be.true

    await fs.promises.unlink( file ).catch( () => {} )
  } )

  it( "reports a file it cannot open rather than failing silently", async function() {

    this.timeout( 6000 )
    this.slow( 3000 )

    const { armed, event } = await capture( "/proc/nope/cannot.pcap" )

    /* a fixed reason, with the OS error beside it rather than inside it */
    expect( armed ).to.be.true
    expect( event.reason ).to.equal( "open-failed" )
    expect( event.event ).to.equal( "finished.open-failed" )
    expect( event.error ).to.be.a( "string" ).and.not.be.empty
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

    /* Real calls are bridged, not echoed - the mixer sends on its own path,
       and that path matters most in production. */
    const filea = tmpfile( "mixa" )
    const fileb = tmpfile( "mixb" )

    const endpointa = dgram.createSocket( "udp4" )
    const endpointb = dgram.createSocket( "udp4" )
    endpointa.on( "message", () => {} )
    endpointb.on( "message", () => {} )

    await /** @type { Promise< void > } */ ( new Promise( ( r ) => { endpointa.bind( r ) } ) )
    await /** @type { Promise< void > } */ ( new Promise( ( r ) => { endpointb.bind( r ) } ) )

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
    const framesa = rtpframes( await readpcap( filea ) )
    const framesb = rtpframes( await readpcap( fileb ) )
    expect( framesa.length ).to.be.above( 40 )
    expect( framesb.length ).to.be.above( 40 )

    framesa.forEach( ( f ) => { expect( f.ipv4.udp.data.length ).to.equal( 172 ) } )
    framesb.forEach( ( f ) => { expect( f.ipv4.udp.data.length ).to.equal( 172 ) } )

    await fs.promises.unlink( filea ).catch( () => {} )
    await fs.promises.unlink( fileb ).catch( () => {} )
  } )

  it( "captures a dtls-srtp leg as it was on the wire", async function() {

    this.timeout( 10000 )
    this.slow( 6000 )

    /* The taps sit at the socket, outside the crypto boundary, so a secure
       leg captures as SRTP: the handshake that keyed it, then media whose
       payload is ciphertext (~8 bits/byte, against far less for g711) and
       carries the auth tag. So a capture holds no call audio or digits. The
       RTP header stays in the clear, which is what Wireshark's loss and
       jitter analysis reads. */
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

    const allframes = await readpcap( file )

    /* the DTLS handshake (content types 20-63) is in the file */
    const dtls = allframes.filter( ( f ) => 20 <= f.ipv4.udp.data[ 0 ] && 63 >= f.ipv4.udp.data[ 0 ] )
    expect( dtls.length ).to.be.above( 0 )

    const frames = rtpframes( allframes )
    expect( frames.length ).to.be.above( 70 )

    /* the srtp auth tag (80 bits) makes the wire form 10 bytes longer */
    frames.forEach( ( f ) => { expect( f.ipv4.udp.data.length ).to.equal( 182 ) } )

    const payload = Buffer.concat( frames.map( ( f ) => f.ipv4.udp.data.subarray( 12, 172 ) ) )
    expect( entropy( payload ) ).to.be.above( 7,
      "captured payload looks like cleartext - the tap is inside the srtp boundary" )

    /* and the header Wireshark analyses is readable: one stream, in order */
    const inbound = frames.filter( ( f ) => f.ipv4.udp.dstport === channelb.local.port )
    expect( new Set( inbound.map( ( f ) => f.ipv4.udp.data.readUInt32BE( 8 ) ) ).size ).to.equal( 1 )
    const sns = inbound.map( ( f ) => f.ipv4.udp.data.readUInt16BE( 2 ) )
    for( let i = 1; i < sns.length; i++ ) expect( ( sns[ i ] - sns[ i - 1 ] + 65536 ) % 65536 ).to.equal( 1 )

    await fs.promises.unlink( file ).catch( () => {} )
    await fs.promises.unlink( "/tmp/pcapringing.wav" ).catch( () => {} )
  } )

  it( "captures packets as they arrive, before the jitter buffer drops any", async function() {

    this.timeout( 6000 )
    this.slow( 3000 )

    /* every packet sent twice: the jitter buffer drops each duplicate, but
       they were on the wire, so the capture must have them */
    const file = tmpfile( "dupes" )
    const server = dgram.createSocket( "udp4" )
    server.on( "message", () => {} )
    await /** @type { Promise< void > } */ ( new Promise( ( r ) => { server.bind( r ) } ) )

    const events = []
    const channel = await projectrtp.openchannel( {
      "remote": { "address": "127.0.0.1", "port": server.address().port, "codec": 0 }
    }, ( d ) => { events.push( d ) } )
    expect( channel.pcap( { "file": file } ) ).to.be.true

    for( let i = 0; 30 > i; i++ ) {
      sendpk( i, channel.local.port, server )
      sendpk( i, channel.local.port, server )
    }
    await new Promise( ( r ) => { setTimeout( r, 900 ) } )
    const peerport = server.address().port
    channel.close()
    await new Promise( ( r ) => { setTimeout( r, 300 ) } )
    server.close()

    const inbound = rtpframes( await readpcap( file ) )
      .filter( ( f ) => f.ipv4.udp.dstport === channel.local.port )
    expect( inbound.length ).to.equal( 60 )

    /* from the peer's real address and port */
    inbound.forEach( ( f ) => {
      expect( f.ipv4.src ).to.equal( "127.0.0.1" )
      expect( f.ipv4.udp.srcport ).to.equal( peerport )
    } )

    await fs.promises.unlink( file ).catch( () => {} )
  } )

  it( "captures rtcp that arrives on the control port", async function() {

    this.timeout( 6000 )
    this.slow( 3000 )

    const file = tmpfile( "rtcp" )
    const server = dgram.createSocket( "udp4" )
    server.on( "message", () => {} )
    await /** @type { Promise< void > } */ ( new Promise( ( r ) => { server.bind( r ) } ) )

    const channel = await projectrtp.openchannel( {
      "remote": { "address": "127.0.0.1", "port": server.address().port, "codec": 0 }
    }, () => {} )
    expect( channel.pcap( { "file": file } ) ).to.be.true

    /* a receiver report, to the RTP port + 1 */
    const rr = Buffer.alloc( 8 )
    rr[ 0 ] = 0x80
    rr[ 1 ] = 201
    rr.writeUInt16BE( 1, 2 )
    rr.writeUInt32BE( 0x12345678, 4 )
    server.send( rr, channel.local.port + 1, "127.0.0.1" )

    await new Promise( ( r ) => { setTimeout( r, 300 ) } )
    channel.close()
    await new Promise( ( r ) => { setTimeout( r, 300 ) } )
    server.close()

    const frames = await readpcap( file )
    const received = frames.filter( ( f ) =>
      f.ipv4.udp.dstport === channel.local.port + 1 && isrtcp( f.ipv4.udp.data ) )
    expect( received.length ).to.equal( 1 )
    expect( received[ 0 ].ipv4.udp.data.readUInt32BE( 4 ) ).to.equal( 0x12345678 )

    await fs.promises.unlink( file ).catch( () => {} )
  } )

  it( "captures a relay (video) leg, both directions", async function() {

    this.timeout( 6000 )
    this.slow( 3000 )

    /* relay legs forward without the tick, so they were the path a capture
       taken inside the tick could not see */
    const file = tmpfile( "relay" )
    const bind = () => new Promise( ( r ) => {
      const sock = dgram.createSocket( "udp4" )
      sock.on( "message", () => {} )
      sock.bind( 0, "127.0.0.1", () => r( sock ) )
    } )
    const alice = await bind()
    const bob = await bind()

    const events = []
    const a = await projectrtp.openchannel(
      { "relay": true, "forcelocal": true, "remote": { "address": "127.0.0.1", "port": alice.address().port, "codec": 96 } },
      ( d ) => { events.push( d ) } )
    const b = await projectrtp.openchannel(
      { "relay": true, "forcelocal": true, "remote": { "address": "127.0.0.1", "port": bob.address().port, "codec": 96 } },
      () => {} )
    expect( a.mix( b ) ).to.be.true
    expect( a.pcap( { "file": file } ) ).to.be.true

    /* alice sends into leg a, which forwards to bob through leg b; bob sends
       into leg b, which forwards to alice through leg a */
    const video = ( sn ) => {
      const p = Buffer.alloc( 12 + 1000 ).fill( sn & 0xff )
      p[ 0 ] = 0x80
      p[ 1 ] = 96
      p.writeUInt16BE( sn, 2 )
      p.writeUInt32BE( sn * 3000, 4 )
      p.writeUInt32BE( 0x11223344, 8 )
      return p
    }
    for( let i = 0; 10 > i; i++ ) {
      alice.send( video( i ), a.local.port, "127.0.0.1" )
      bob.send( video( 100 + i ), b.local.port, "127.0.0.1" )
    }
    await new Promise( ( r ) => { setTimeout( r, 300 ) } )
    const aliceport = alice.address().port

    a.close()
    b.close()
    await new Promise( ( r ) => { setTimeout( r, 300 ) } )
    alice.close()
    bob.close()

    const pcap = events.find( ( e ) => "pcap" === e.action )
    expect( pcap.packets ).to.be.above( 0 )

    const frames = rtpframes( await readpcap( file ) )
    const fromalice = frames.filter( ( f ) => f.ipv4.udp.srcport === aliceport )
    const toalice = frames.filter( ( f ) => f.ipv4.udp.dstport === aliceport )
    expect( fromalice.length ).to.equal( 10 )
    expect( toalice.length ).to.equal( 10 )
    fromalice.forEach( ( f ) => { expect( f.ipv4.udp.data.length ).to.equal( 1012 ) } )

    await fs.promises.unlink( file ).catch( () => {} )
  } )
} )
