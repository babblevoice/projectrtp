/*
Relay mode (video) — a channel opened with { relay: true } forwards RTP
between two legs without ticking, decoding or re-timestamping. These tests
drive two plain-RTP relay legs from two dgram sockets standing in for the
endpoints:

  alice ── A(relay) ══ mix ══ B(relay) ── bob

What is pinned, and why:

- Timestamp and marker bit survive verbatim (one video frame's fragments
  share a timestamp; the marker is end-of-frame — rewriting either breaks
  decode), while SSRC is re-originated per leg. Sequence numbers are the
  source's plus a per-leg offset, so upstream loss and reordering still
  reach the receiver as gaps / out-of-order numbers (a plain arrival-order
  counter hides them: no NACK, broken frame assembly).
- A burst far above the audio tick's one-packet-per-20ms ceiling arrives
  intact. The audio path could never exceed 50 pps (and its jitter buffer
  holds 32 packets), so this failing means someone routed relay media back
  through the tick.
- RTCP feedback (a PLI from one receiver) surfaces as a PLI to the OTHER
  leg's endpoint — the keyframe has to come from the source, which sits
  behind the peer leg.
- A browser's PLI rides INSIDE a compound behind a leading RR/SR (RFC 3550
  §6.1), so the compound [RR][PLI] shape is tested, not just a bare PLI —
  a bare 206-first datagram takes a different, trivial demux branch.
- A NACK is passed through to the source's endpoint, in the source's own
  SSRC and sequence series, and asks for no keyframe: gaps pass through,
  so receivers NACK on ordinary loss and a NACK-as-PLI was a keyframe storm.
- DTLS-SRTP legs: media is withheld (both directions) until keys exist,
  and media plus a keyframe request cross an SRTP/SRTCP hop intact.
  (The SRTCP-protected [RR][PLI] compound — decrypt once, then scan — is
  pinned in recv_loop.rs's relay_secure unit tests: JS has no DTLS stack
  to key an endpoint with.)
- Groups (N-party): a relay leg forwards to every other leg in its mix
  group, each receiving leg gives each source its own SSRC (the first one
  being the leg's own, so 2-party is one SSRC per direction), a keyframe
  request is routed to the source whose SSRC it names, and unmix stops
  forwarding both ways — no stale peer after re-pairing.
- livestats() reads live — the "registered but no media flowing" detector
  needs counters while the channel is open, not on close. in.count is raw
  (pre-decrypt) arrivals; in.accepted / in.rtcp are what we could
  authenticate and are what keeps a leg from idling out (RTCP alone does:
  camera off), while in.prekey / in.decryptfailed expose a leg that
  receives packets but never keyed.
- Abuse limits (relaylimits): a source over its leg's bitrate has packets
  dropped before fan-out and counted in livestats().in.ratedropped; the
  defaults apply when relaylimits is absent (protection must not depend on
  the caller remembering it) yet admit an honest keyframe burst; 0 turns a
  limit off; and a VP8 keyframe over max-fs is refused whole
  (in.oversizedropped) until a keyframe within it arrives. The token-bucket,
  frame-rate and SPS/keyframe parsing detail is pinned in the Rust unit
  tests (relay_limits.rs, video_dims.rs).
*/

const expect = require( "chai" ).expect
const dgram = require( "dgram" )

const projectrtp = require( "../../index" ).projectrtp

before( () => { projectrtp.run() } )

/**
 * Build an RTP packet buffer.
 * @param { object } opts
 * @returns { Buffer }
 */
function rtppacket( { pt = 96, sn = 0, ts = 0, ssrc = 0x11223344, marker = false, payloadsize = 1000, fill = 0 } ) {
  const header = Buffer.alloc( 12 )
  header[ 0 ] = 0x80
  header[ 1 ] = ( marker ? 0x80 : 0 ) | ( pt & 0x7f )
  header.writeUInt16BE( sn & 0xffff, 2 )
  header.writeUInt32BE( ts >>> 0, 4 )
  header.writeUInt32BE( ssrc >>> 0, 8 )
  return Buffer.concat( [ header, Buffer.alloc( payloadsize ).fill( fill ) ] )
}

/**
 * RFC 4585 PSFB PLI.
 * @param { number } senderssrc
 * @param { number } mediassrc
 * @returns { Buffer }
 */
function plipacket( senderssrc, mediassrc ) {
  const b = Buffer.alloc( 12 )
  b[ 0 ] = 0x81
  b[ 1 ] = 206
  b.writeUInt16BE( 2, 2 )
  b.writeUInt32BE( senderssrc >>> 0, 4 )
  b.writeUInt32BE( mediassrc >>> 0, 8 )
  return b
}

/**
 * RFC 3550 receiver report with no report blocks.
 * @param { number } senderssrc
 * @returns { Buffer }
 */
function rrpacket( senderssrc ) {
  const b = Buffer.alloc( 8 )
  b[ 0 ] = 0x80
  b[ 1 ] = 201
  b.writeUInt16BE( 1, 2 )
  b.writeUInt32BE( senderssrc >>> 0, 4 )
  return b
}

/**
 * Is this datagram a PSFB PLI (PT 206, FMT 1)?
 * @param { Buffer } m
 * @returns { boolean }
 */
function ispli( m ) {
  return 12 <= m.length && 206 === m[ 1 ] && 1 === ( m[ 0 ] & 0x1f )
}

/**
 * Collect channel close promises.
 * @returns { object }
 */
function closetracker() {
  const closed = []
  const mkclose = () => {
    let done
    closed.push( new Promise( ( r ) => { done = r } ) )
    return ( d ) => { if( "close" === d.action ) done() }
  }
  return { closed, mkclose }
}

/**
 * Bind a dgram socket on an OS-assigned port and resolve once listening.
 * @returns { Promise< object > }
 */
function bindsocket() {
  return new Promise( ( resolve ) => {
    const s = dgram.createSocket( "udp4" )
    s.bind( 0, "127.0.0.1", () => resolve( s ) )
  } )
}

/**
 * Open a relay pair wired to the two endpoint sockets and mix them.
 * @param { object } alice
 * @param { object } bob
 * @returns { Promise< object > }
 */
async function openrelaypair( alice, bob ) {
  const closed = []
  const mkclose = () => {
    let done
    closed.push( new Promise( ( r ) => { done = r } ) )
    return ( d ) => { if( "close" === d.action ) done() }
  }

  const a = await projectrtp.openchannel(
    { "relay": true, "forcelocal": true, "remote": { "address": "127.0.0.1", "port": alice.address().port, "codec": 96 } },
    mkclose() )
  const b = await projectrtp.openchannel(
    { "relay": true, "forcelocal": true, "remote": { "address": "127.0.0.1", "port": bob.address().port, "codec": 96 } },
    mkclose() )

  expect( a.mix( b ) ).to.be.true

  return { a, b, closed }
}

describe( "rtprelay", function() {

  it( "forwards RTP preserving timestamp, marker and payload; re-originates ssrc", async function() {

    const alice = await bindsocket()
    const bob = await bindsocket()
    const received = []
    bob.on( "message", ( m ) => received.push( Buffer.from( m ) ) )

    const { a, b, closed } = await openrelaypair( alice, bob )

    /* two fragments of one "frame" (shared ts, marker on the last), then
       the first fragment of the next frame 3000 ticks (90kHz) later */
    const packets = [
      { sn: 1000, ts: 123450, marker: false, fill: 1 },
      { sn: 1001, ts: 123450, marker: true, fill: 2 },
      { sn: 1002, ts: 126450, marker: false, fill: 3 },
    ]
    for( const p of packets )
      alice.send( rtppacket( { ...p, payloadsize: 1200 } ), a.local.port, "127.0.0.1" )

    await new Promise( ( resolve ) => setTimeout( resolve, 300 ) )

    expect( received.length ).to.equal( 3 )
    for( let i = 0; 3 > i; i++ ) {
      const r = received[ i ]
      expect( r[ 1 ] & 0x7f ).to.equal( 96 )
      expect( !!( r[ 1 ] & 0x80 ) ).to.equal( packets[ i ].marker )
      expect( r.readUInt32BE( 4 ) ).to.equal( packets[ i ].ts )
      /* ssrc is the B leg's own, not alice's */
      expect( r.readUInt32BE( 8 ) ).to.equal( b.local.ssrc )
      expect( r.readUInt32BE( 8 ) ).to.not.equal( 0x11223344 )
      expect( r[ 12 ] ).to.equal( packets[ i ].fill )
      expect( r.length ).to.equal( 12 + 1200 )
    }
    /* contiguous in, contiguous out */
    const sns = received.map( ( r ) => r.readUInt16BE( 2 ) )
    expect( sns[ 1 ] ).to.equal( ( sns[ 0 ] + 1 ) & 0xffff )
    expect( sns[ 2 ] ).to.equal( ( sns[ 0 ] + 2 ) & 0xffff )

    a.close()
    b.close()
    await Promise.all( closed )
    alice.close()
    bob.close()
  } )

  it( "keeps upstream loss and reordering visible in the sequence numbers", async function() {

    const alice = await bindsocket()
    const bob = await bindsocket()
    const received = []
    bob.on( "message", ( m ) => received.push( Buffer.from( m ) ) )

    const { a, b, closed } = await openrelaypair( alice, bob )

    /* 65534 arrives, 65535 and 0 are lost across the wrap, then 2 overtakes
       1. Paced so arrival order at the relay is the send order. */
    const sent = [ 65534, 1, 3, 2 ]
    for( const sn of sent ) {
      alice.send( rtppacket( { sn, ts: 3000 * sn, fill: sn & 0xff } ), a.local.port, "127.0.0.1" )
      await new Promise( ( resolve ) => setTimeout( resolve, 20 ) )
    }

    await new Promise( ( resolve ) => setTimeout( resolve, 300 ) )

    expect( received ).to.have.lengthOf( 4 )
    const base = received[ 0 ].readUInt16BE( 2 )
    const deltas = received.map( ( r ) => ( r.readUInt16BE( 2 ) - base ) & 0xffff )
    /* the same spacing as the source: a gap of 3 across the wrap, then 3, 2 */
    expect( deltas ).to.deep.equal( [ 0, 3, 5, 4 ] )
    /* and each forwarded packet is the one that was sent in that slot */
    expect( received.map( ( r ) => r[ 12 ] ) ).to.deep.equal( sent.map( ( sn ) => sn & 0xff ) )

    a.close()
    b.close()
    await Promise.all( closed )
    alice.close()
    bob.close()
  } )

  it( "a second SSRC from the same source never stops its main stream", async function() {

    /* RTX, FEC, a simulcast layer or one stray packet under another SSRC
       used to mark the main stream as "previous" (A, B, A) and drop it for
       the rest of the call */
    const alice = await bindsocket()
    const bob = await bindsocket()
    const received = []
    bob.on( "message", ( m ) => received.push( Buffer.from( m ) ) )

    const { a, b, closed } = await openrelaypair( alice, bob )

    const main = 0x11223344
    const other = 0x55667788
    const sent = [
      { ssrc: main, sn: 10 }, { ssrc: other, sn: 500 }, { ssrc: main, sn: 11 },
      { ssrc: main, sn: 12 }, { ssrc: other, sn: 501 }, { ssrc: main, sn: 13 },
    ]
    for( const p of sent ) {
      alice.send( rtppacket( { ...p, fill: p.sn & 0xff } ), a.local.port, "127.0.0.1" )
      await new Promise( ( resolve ) => setTimeout( resolve, 20 ) )
    }
    await new Promise( ( resolve ) => setTimeout( resolve, 300 ) )

    expect( received ).to.have.lengthOf( 6 )
    const mains = received.filter( ( r ) => r.readUInt32BE( 8 ) === b.local.ssrc )
    const others = received.filter( ( r ) => r.readUInt32BE( 8 ) !== b.local.ssrc )
    /* the main stream is whole, on the leg's own SSRC, in one series */
    expect( mains.map( ( r ) => r[ 12 ] ) ).to.deep.equal( [ 10, 11, 12, 13 ] )
    const base = mains[ 0 ].readUInt16BE( 2 )
    expect( mains.map( ( r ) => ( r.readUInt16BE( 2 ) - base ) & 0xffff ) ).to.deep.equal( [ 0, 1, 2, 3 ] )
    /* the other stream rides an outbound SSRC of its own */
    expect( others.map( ( r ) => r[ 12 ] ) ).to.deep.equal( [ 500 & 0xff, 501 & 0xff ] )
    expect( others[ 0 ].readUInt32BE( 8 ) ).to.equal( others[ 1 ].readUInt32BE( 8 ) )
    expect( others[ 0 ].readUInt32BE( 8 ) ).to.not.equal( other )

    a.close()
    b.close()
    await Promise.all( closed )
    alice.close()
    bob.close()
  } )

  it( "maps the payload type into the receiving leg's and drops PTs with no mapping", async function() {

    /* every forwarded packet used to be re-stamped to the receiver's one PT,
       so an RTX / FEC / second-codec packet went out labelled as the video
       codec */
    const alice = await bindsocket()
    const bob = await bindsocket()
    const received = []
    bob.on( "message", ( m ) => received.push( Buffer.from( m ) ) )
    const { closed, mkclose } = closetracker()

    const a = await projectrtp.openchannel(
      { "relay": true, "forcelocal": true, "remote": { "address": "127.0.0.1", "port": alice.address().port, "codec": 96 } },
      mkclose() )
    const b = await projectrtp.openchannel(
      { "relay": true, "forcelocal": true, "remote": { "address": "127.0.0.1", "port": bob.address().port, "codec": 100 } },
      mkclose() )
    expect( a.mix( b ) ).to.be.true

    const sent = [ { pt: 96, sn: 1 }, { pt: 97, sn: 2, ssrc: 0x99 }, { pt: 96, sn: 3 } ]
    for( const p of sent ) {
      alice.send( rtppacket( { ...p, fill: p.sn } ), a.local.port, "127.0.0.1" )
      await new Promise( ( resolve ) => setTimeout( resolve, 20 ) )
    }
    await new Promise( ( resolve ) => setTimeout( resolve, 300 ) )

    expect( received.map( ( r ) => [ r[ 1 ] & 0x7f, r[ 12 ] ] ) ).to.deep.equal( [ [ 100, 1 ], [ 100, 3 ] ] )
    expect( b.livestats().out ).to.include( { count: 2, ptdropped: 1 } )

    a.close()
    b.close()
    await Promise.all( closed )
    alice.close()
    bob.close()
  } )

  it( "maps several codecs by label with remote.codecs (openchannel and remote())", async function() {

    const alice = await bindsocket()
    const bob = await bindsocket()
    const received = []
    bob.on( "message", ( m ) => received.push( Buffer.from( m ) ) )
    const { closed, mkclose } = closetracker()

    const a = await projectrtp.openchannel(
      { "relay": true, "forcelocal": true, "remote": {
        "address": "127.0.0.1", "port": alice.address().port,
        "codec": 96, "codecs": { "vp8": 96, "h264": 102 } } },
      mkclose() )
    const b = await projectrtp.openchannel( { "relay": true, "forcelocal": true }, mkclose() )
    expect( b.remote( {
      "address": "127.0.0.1", "port": bob.address().port,
      "codec": 98, "codecs": { "VP8": 98, "H264": 104 } } ) ).to.be.true
    expect( a.mix( b ) ).to.be.true
    await new Promise( ( resolve ) => setTimeout( resolve, 50 ) )

    /* 111 is declared by neither leg */
    const sent = [ { pt: 96, sn: 1 }, { pt: 102, sn: 2, ssrc: 0x77 }, { pt: 111, sn: 3, ssrc: 0x88 } ]
    for( const p of sent ) {
      alice.send( rtppacket( { ...p, fill: p.sn } ), a.local.port, "127.0.0.1" )
      await new Promise( ( resolve ) => setTimeout( resolve, 20 ) )
    }
    await new Promise( ( resolve ) => setTimeout( resolve, 300 ) )

    expect( received.map( ( r ) => [ r[ 1 ] & 0x7f, r[ 12 ] ] ) ).to.deep.equal( [ [ 98, 1 ], [ 104, 2 ] ] )
    expect( b.livestats().out ).to.include( { count: 2, ptdropped: 1 } )

    a.close()
    b.close()
    await Promise.all( closed )
    alice.close()
    bob.close()
  } )

  it( "carries a burst far above the audio tick's 50 pps ceiling", async function() {

    const alice = await bindsocket()
    const bob = await bindsocket()
    let received = 0
    bob.on( "message", () => received++ )

    const { a, b, closed } = await openrelaypair( alice, bob )

    /* 200 near-MTU packets in ~100ms (2000 pps) — a keyframe burst, so
       one frame (one timestamp), ~220KB: inside the default relaylimits
       burst, which must admit exactly this. The audio path could deliver
       at most ~5 in that window (one per 20ms tick) and its 32-slot
       jitter buffer would shed the rest. Sent in paced batches of 10 per
       5ms so the kernel's default UDP receive buffer (~200KB) isn't the
       thing under test. */
    const total = 200
    for( let i = 0; total > i; i += 10 ) {
      for( let j = i; i + 10 > j; j++ )
        alice.send( rtppacket( { sn: j, ts: 90000, payloadsize: 1100 } ), a.local.port, "127.0.0.1" )
      await new Promise( ( resolve ) => setTimeout( resolve, 5 ) )
    }

    await new Promise( ( resolve ) => setTimeout( resolve, 1000 ) )

    expect( received ).to.be.above( 150 )

    /* livestats reads live, while the channel is open */
    const astats = a.livestats()
    const bstats = b.livestats()
    expect( astats.relay ).to.be.true
    expect( astats.in.count ).to.be.above( 150 )
    expect( astats.in.accepted ).to.equal( astats.in.count )
    expect( astats.in.prekey ).to.equal( 0 )
    expect( astats.in.decryptfailed ).to.equal( 0 )
    expect( astats.in.ratedropped ).to.equal( 0 )
    expect( astats.in.framedropped ).to.equal( 0 )
    expect( bstats.out.count ).to.equal( received )

    a.close()
    b.close()
    await Promise.all( closed )
    alice.close()
    bob.close()
  } )

  it( "relays a keyframe request (PLI) to the peer leg's endpoint", async function() {

    const alice = await bindsocket()
    const bob = await bindsocket()
    const alicertcp = []
    alice.on( "message", ( m ) => {
      /* PSFB PLI: PT 206, FMT 1 */
      if( 12 <= m.length && 206 === m[ 1 ] && 1 === ( m[ 0 ] & 0x1f ) ) alicertcp.push( Buffer.from( m ) )
    } )

    const { a, b, closed } = await openrelaypair( alice, bob )

    /* alice must have sent media first — a PLI is about a latched source,
       and it also latches alice's address for A's outbound */
    alice.send( rtppacket( { sn: 1, ts: 0, ssrc: 0xcafebabe } ), a.local.port, "127.0.0.1" )
    await new Promise( ( resolve ) => setTimeout( resolve, 200 ) )

    /* bob (the receiver of the forwarded stream) asks for a keyframe */
    bob.send( plipacket( 0x1, b.local.ssrc ), b.local.port, "127.0.0.1" )

    await new Promise( ( resolve ) => setTimeout( resolve, 500 ) )

    expect( alicertcp.length ).to.be.above( 0 )
    /* the PLI names A's ssrc as packet sender and alice's stream as media */
    expect( alicertcp[ 0 ].readUInt32BE( 4 ) ).to.equal( a.local.ssrc )
    expect( alicertcp[ 0 ].readUInt32BE( 8 ) ).to.equal( 0xcafebabe )

    a.close()
    b.close()
    await Promise.all( closed )
    alice.close()
    bob.close()
  } )

  it( "passes a NACK through to the source leg's endpoint instead of asking for a keyframe", async function() {

    /* the relay keeps the source's sequence gaps, so a receiver NACKs on
       ordinary loss; turning each NACK into a PLI asked the source for a
       keyframe every 300 ms. The source must get the NACK itself, in its own
       SSRC and sequence series, and retransmit. */
    const alice = await bindsocket()
    const bob = await bindsocket()
    const alicenack = []
    const alicepli = []
    alice.on( "message", ( m ) => {
      if( ispli( m ) ) alicepli.push( Buffer.from( m ) )
      if( 16 <= m.length && 205 === m[ 1 ] && 1 === ( m[ 0 ] & 0x1f ) ) alicenack.push( Buffer.from( m ) )
    } )

    const { a, b, closed } = await openrelaypair( alice, bob )

    alice.send( rtppacket( { sn: 500, ssrc: 0xcafebabe } ), a.local.port, "127.0.0.1" )
    alice.send( rtppacket( { sn: 503, ssrc: 0xcafebabe } ), a.local.port, "127.0.0.1" )
    /* forwarded, and clear of the join-time PLI rate-limit window */
    await new Promise( ( resolve ) => setTimeout( resolve, 400 ) )
    const plisbefore = alicepli.length

    /* bob lost 501 and 502: NACK pid 501, blp bit 0 (= 502) */
    const nack = Buffer.alloc( 16 )
    nack[ 0 ] = 0x81
    nack[ 1 ] = 205
    nack.writeUInt16BE( 3, 2 )
    nack.writeUInt32BE( 0x1, 4 )
    nack.writeUInt32BE( b.local.ssrc, 8 )
    nack.writeUInt16BE( 501, 12 )
    nack.writeUInt16BE( 1, 14 )
    /* behind an RR as a browser sends it; the immediate repeat asks for
       packets already requested inside the dedupe window, so it must NOT
       reach alice a second time */
    bob.send( Buffer.concat( [ rrpacket( 0x1 ), nack ] ), b.local.port, "127.0.0.1" )
    bob.send( Buffer.concat( [ rrpacket( 0x1 ), nack ] ), b.local.port, "127.0.0.1" )
    /* past the 100 ms window, bare (the feedback demux) — relayed again */
    await new Promise( ( resolve ) => setTimeout( resolve, 250 ) )
    bob.send( nack, b.local.port, "127.0.0.1" )

    await new Promise( ( resolve ) => setTimeout( resolve, 400 ) )

    expect( alicenack.length ).to.equal( 2 )
    for( const n of alicenack ) {
      expect( n.readUInt32BE( 4 ) ).to.equal( a.local.ssrc )
      expect( n.readUInt32BE( 8 ) ).to.equal( 0xcafebabe )
      expect( n.readUInt16BE( 12 ) ).to.equal( 501 )
      expect( n.readUInt16BE( 14 ) ).to.equal( 1 )
    }
    expect( alicepli.length ).to.equal( plisbefore )

    a.close()
    b.close()
    await Promise.all( closed )
    alice.close()
    bob.close()
  } )

  it( "relays a keyframe request riding inside an RTCP compound [RR][PLI]", async function() {

    const alice = await bindsocket()
    const bob = await bindsocket()
    const alicepli = []
    alice.on( "message", ( m ) => { if( ispli( m ) ) alicepli.push( Buffer.from( m ) ) } )

    const { a, b, closed } = await openrelaypair( alice, bob )

    alice.send( rtppacket( { sn: 1, ts: 0, ssrc: 0xcafebabe } ), a.local.port, "127.0.0.1" )
    await new Promise( ( resolve ) => setTimeout( resolve, 200 ) )
    const before = alicepli.length

    /* the browser shape: RR first, the PLI behind it in the same datagram.
       Its second byte is 201, so it is demuxed as ordinary RTCP — only a
       walk of the whole compound finds the PLI. */
    bob.send( Buffer.concat( [ rrpacket( 0x1 ), plipacket( 0x1, b.local.ssrc ) ] ), b.local.port, "127.0.0.1" )

    await new Promise( ( resolve ) => setTimeout( resolve, 500 ) )

    expect( alicepli.length ).to.be.above( before )
    const pli = alicepli[ alicepli.length - 1 ]
    expect( pli.readUInt32BE( 4 ) ).to.equal( a.local.ssrc )
    expect( pli.readUInt32BE( 8 ) ).to.equal( 0xcafebabe )

    /* an RR on its own asks for nothing */
    const afterpli = alicepli.length
    await new Promise( ( resolve ) => setTimeout( resolve, 400 ) ) /* clear the PLI rate limit window */
    bob.send( rrpacket( 0x1 ), b.local.port, "127.0.0.1" )
    await new Promise( ( resolve ) => setTimeout( resolve, 400 ) )
    expect( alicepli.length ).to.equal( afterpli )

    a.close()
    b.close()
    await Promise.all( closed )
    alice.close()
    bob.close()
  } )

  it( "a DTLS-SRTP relay leg withholds media both ways until keyed", async function() {

    /* S is a secure relay leg whose "remote" is a plain socket (eve) that
       never completes DTLS — so S never gets keys. Pair: alice ─ C(clear)
       ══ mix ══ S(dtls) ─ eve. */
    const alice = await bindsocket()
    const eve = await bindsocket()
    const alicegot = []
    const evegot = []
    alice.on( "message", ( m ) => alicegot.push( Buffer.from( m ) ) )
    eve.on( "message", ( m ) => evegot.push( Buffer.from( m ) ) )

    const { closed, mkclose } = closetracker()
    const c = await projectrtp.openchannel(
      { "relay": true, "forcelocal": true, "remote": { "address": "127.0.0.1", "port": alice.address().port, "codec": 96 } },
      mkclose() )
    const s = await projectrtp.openchannel( { "relay": true, "forcelocal": true }, mkclose() )
    expect( s.remote( {
      "address": "127.0.0.1",
      "port": eve.address().port,
      "codec": 96,
      "dtls": { "fingerprint": { "hash": projectrtp.dtls.fingerprint }, "mode": "active" }
    } ) ).to.be.true
    expect( c.mix( s ) ).to.be.true

    /* outbound: alice's media must not leak to eve in the clear */
    for( let i = 0; 10 > i; i++ )
      alice.send( rtppacket( { sn: i, ts: 3000 * i, fill: 0xab } ), c.local.port, "127.0.0.1" )
    /* inbound: unauthenticated cleartext RTP from eve must not reach alice */
    for( let i = 0; 10 > i; i++ )
      eve.send( rtppacket( { sn: i, ts: 3000 * i, fill: 0xcd } ), s.local.port, "127.0.0.1" )

    await new Promise( ( resolve ) => setTimeout( resolve, 500 ) )

    /* eve may see DTLS (first byte 20..63) — never an RTP/RTCP packet */
    expect( evegot.filter( ( m ) => 128 <= m[ 0 ] && 191 >= m[ 0 ] ) ).to.have.lengthOf( 0 )
    expect( alicegot.filter( ( m ) => 12 < m.length && 0xcd === m[ 12 ] ) ).to.have.lengthOf( 0 )
    expect( s.livestats().out.count ).to.equal( 0 )
    expect( c.livestats().out.count ).to.equal( 0 )
    /* S receives packets but cannot authenticate any: raw arrivals move,
       the liveness counters do not — so it idles out rather than looking
       healthy */
    const sstats = s.livestats()
    expect( sstats.in.count ).to.equal( 10 )
    expect( sstats.in.prekey ).to.equal( 10 )
    expect( sstats.in.accepted ).to.equal( 0 )
    expect( sstats.in.rtcp ).to.equal( 0 )

    c.close()
    s.close()
    await Promise.all( closed )
    alice.close()
    eve.close()
  } )

  it( "carries media and a keyframe request across a DTLS-SRTP hop", async function() {

    this.timeout( 6000 )

    /* alice ─ CA(clear) ══ mix ══ SA(dtls) ⇄ SRTP ⇄ SB(dtls) ══ mix ══ CB(clear) ─ bob
       SA encrypts, SB decrypts (fail-closed until keyed); bob's [RR][PLI] crosses back as SRTCP from SB to SA. */
    const alice = await bindsocket()
    const bob = await bindsocket()
    const bobgot = []
    const alicepli = []
    bob.on( "message", ( m ) => { if( 12 < m.length && 96 === ( m[ 1 ] & 0x7f ) ) bobgot.push( Buffer.from( m ) ) } )
    alice.on( "message", ( m ) => { if( ispli( m ) ) alicepli.push( Buffer.from( m ) ) } )

    const { closed, mkclose } = closetracker()
    const clear = ( sock ) => projectrtp.openchannel(
      { "relay": true, "forcelocal": true, "remote": { "address": "127.0.0.1", "port": sock.address().port, "codec": 96 } },
      mkclose() )
    const ca = await clear( alice )
    const cb = await clear( bob )
    const sa = await projectrtp.openchannel( { "relay": true, "forcelocal": true }, mkclose() )
    const sb = await projectrtp.openchannel( { "relay": true, "forcelocal": true }, mkclose() )

    expect( sa.remote( {
      "address": "127.0.0.1",
      "port": sb.local.port,
      "codec": 96,
      "dtls": { "fingerprint": { "hash": sb.local.dtls.fingerprint }, "mode": "active" }
    } ) ).to.be.true
    expect( sb.remote( {
      "address": "127.0.0.1",
      "port": sa.local.port,
      "codec": 96,
      "dtls": { "fingerprint": { "hash": sa.local.dtls.fingerprint }, "mode": "passive" }
    } ) ).to.be.true

    expect( ca.mix( sa ) ).to.be.true
    expect( sb.mix( cb ) ).to.be.true

    /* settle the handshake — media sent before it is withheld, not leaked */
    await new Promise( ( resolve ) => setTimeout( resolve, 800 ) )

    const packets = [
      { sn: 500, ts: 90000, marker: false, fill: 1 },
      { sn: 501, ts: 90000, marker: true, fill: 2 },
      { sn: 502, ts: 93000, marker: false, fill: 3 },
    ]
    for( const p of packets )
      alice.send( rtppacket( { ...p, ssrc: 0xcafebabe, payloadsize: 900 } ), ca.local.port, "127.0.0.1" )

    await new Promise( ( resolve ) => setTimeout( resolve, 400 ) )

    /* plaintext at bob: SRTP encrypt at SA and decrypt at SB both ran */
    expect( bobgot ).to.have.lengthOf( 3 )
    for( let i = 0; 3 > i; i++ ) {
      const r = bobgot[ i ]
      expect( r.length ).to.equal( 12 + 900 )
      expect( !!( r[ 1 ] & 0x80 ) ).to.equal( packets[ i ].marker )
      expect( r.readUInt32BE( 4 ) ).to.equal( packets[ i ].ts )
      expect( r.readUInt32BE( 8 ) ).to.equal( cb.local.ssrc )
      expect( r[ 12 ] ).to.equal( packets[ i ].fill )
      expect( r[ 12 + 899 ] ).to.equal( packets[ i ].fill )
    }
    expect( sb.livestats().in.count ).to.be.at.least( 3 )

    /* bob asks for a keyframe; it has to cross the SRTCP hop to reach alice */
    await new Promise( ( resolve ) => setTimeout( resolve, 400 ) ) /* clear the PLI rate limit window */
    const before = alicepli.length
    bob.send( Buffer.concat( [ rrpacket( 0x1 ), plipacket( 0x1, cb.local.ssrc ) ] ), cb.local.port, "127.0.0.1" )

    await new Promise( ( resolve ) => setTimeout( resolve, 600 ) )

    expect( alicepli.length ).to.be.above( before )
    const pli = alicepli[ alicepli.length - 1 ]
    expect( pli.readUInt32BE( 4 ) ).to.equal( ca.local.ssrc )
    expect( pli.readUInt32BE( 8 ) ).to.equal( 0xcafebabe )

    for( const ch of [ ca, sa, sb, cb ] ) ch.close()
    await Promise.all( closed )
    alice.close()
    bob.close()
  } )

  describe( "groups", function() {

    /**
     * Three endpoints, three relay legs, one group via mix( a, b ), mix( a, c ).
     * @returns { Promise< object > }
     */
    async function threeway() {
      const socks = [ await bindsocket(), await bindsocket(), await bindsocket() ]
      const got = socks.map( () => [] )
      socks.forEach( ( s, i ) => s.on( "message", ( m ) => got[ i ].push( Buffer.from( m ) ) ) )
      const { closed, mkclose } = closetracker()
      const legs = []
      for( const s of socks )
        legs.push( await projectrtp.openchannel(
          { "relay": true, "forcelocal": true, "remote": { "address": "127.0.0.1", "port": s.address().port, "codec": 96 } },
          mkclose() ) )
      expect( legs[ 0 ].mix( legs[ 1 ] ) ).to.be.true
      expect( legs[ 0 ].mix( legs[ 2 ] ) ).to.be.true
      const done = async () => {
        for( const l of legs ) l.close()
        await Promise.all( closed )
        for( const s of socks ) s.close()
      }
      return { socks, got, legs, done }
    }

    const media = ( got ) => got.filter( ( m ) => 12 < m.length && 96 === ( m[ 1 ] & 0x7f ) )
    const wait = ( ms ) => new Promise( ( resolve ) => setTimeout( resolve, ms ) )

    it( "fans each source out to every other leg, one SSRC per source", async function() {
      const { socks, got, legs, done } = await threeway()
      const [ alice, bob ] = socks
      const [ a, b, c ] = legs

      for( let i = 0; 5 > i; i++ ) {
        alice.send( rtppacket( { sn: 100 + i, ts: 3000 * i, ssrc: 0xa11ce, fill: 0xaa } ), a.local.port, "127.0.0.1" )
        bob.send( rtppacket( { sn: 900 + i, ts: 3000 * i, ssrc: 0xb0b, fill: 0xbb } ), b.local.port, "127.0.0.1" )
      }
      await wait( 300 )

      const [ atalice, atbob, atcarol ] = got.map( media )
      /* nobody hears themselves; alice gets bob, bob gets alice */
      expect( atalice.map( ( m ) => m[ 12 ] ) ).to.deep.equal( Array( 5 ).fill( 0xbb ) )
      expect( atbob.map( ( m ) => m[ 12 ] ) ).to.deep.equal( Array( 5 ).fill( 0xaa ) )
      expect( atalice.every( ( m ) => m.readUInt32BE( 8 ) === a.local.ssrc ) ).to.be.true
      expect( atbob.every( ( m ) => m.readUInt32BE( 8 ) === b.local.ssrc ) ).to.be.true
      /* carol gets both, on two distinct SSRCs, each with its source's own
         sequence spacing */
      expect( atcarol ).to.have.lengthOf( 10 )
      const byssrc = new Map()
      for( const m of atcarol ) {
        const k = m.readUInt32BE( 8 )
        if( !byssrc.has( k ) ) byssrc.set( k, [] )
        byssrc.get( k ).push( m )
      }
      expect( byssrc.size ).to.equal( 2 )
      for( const stream of byssrc.values() ) {
        expect( stream ).to.have.lengthOf( 5 )
        expect( new Set( stream.map( ( m ) => m[ 12 ] ) ).size ).to.equal( 1 )
        const base = stream[ 0 ].readUInt16BE( 2 )
        expect( stream.map( ( m ) => ( m.readUInt16BE( 2 ) - base ) & 0xffff ) ).to.deep.equal( [ 0, 1, 2, 3, 4 ] )
      }
      expect( c.livestats().out.count ).to.equal( 10 )

      await done()
    } )

    it( "livestats lists each leg's outbound streams by source uuid", async function() {
      /* the relay invents an outbound SSRC per source stream; signalling can
         only announce it (a=ssrc / msid) to a receiver if it can read it */
      const { socks, got, legs, done } = await threeway()
      const [ alice, bob ] = socks
      const [ a, b, c ] = legs

      expect( c.livestats().streams ).to.deep.equal( [] )
      for( let i = 0; 3 > i; i++ ) {
        alice.send( rtppacket( { sn: 100 + i, ssrc: 0xa11ce, fill: 0xaa } ), a.local.port, "127.0.0.1" )
        bob.send( rtppacket( { sn: 900 + i, ssrc: 0xb0b, fill: 0xbb } ), b.local.port, "127.0.0.1" )
      }
      await wait( 300 )

      const atcarol = media( got[ 2 ] )
      const ssrcof = ( fill ) => atcarol.find( ( m ) => fill === m[ 12 ] ).readUInt32BE( 8 )
      const streams = c.livestats().streams
      const sorted = ( v ) => [ ...v ].sort( ( x, y ) => x.source < y.source ? -1 : 1 )
      expect( sorted( streams ) ).to.deep.equal( sorted( [
        { "source": a.uuid, "ssrc": ssrcof( 0xaa ), "sourcessrc": 0xa11ce, "pt": 96 },
        { "source": b.uuid, "ssrc": ssrcof( 0xbb ), "sourcessrc": 0xb0b, "pt": 96 },
      ] ) )
      /* the first source seen gets the leg's own SSRC */
      expect( streams.map( ( st ) => st.ssrc ) ).to.include( c.local.ssrc )
      expect( a.livestats().streams ).to.deep.equal( [
        { "source": b.uuid, "ssrc": a.local.ssrc, "sourcessrc": 0xb0b, "pt": 96 } ] )

      /* a source that leaves drops out of the mapping */
      expect( b.unmix() ).to.be.true
      expect( c.livestats().streams.map( ( st ) => st.source ) ).to.deep.equal( [ a.uuid ] )

      await done()
    } )

    it( "unmix stops forwarding both ways (no stale peer after re-pairing)", async function() {
      const { socks, got, legs, done } = await threeway()
      const [ alice, bob, carol ] = socks
      const [ a, b, c ] = legs

      /* bob leaves; alice and carol stay paired */
      expect( b.unmix() ).to.be.true
      await wait( 50 )
      bob.send( rtppacket( { sn: 1, fill: 0xbb } ), b.local.port, "127.0.0.1" )
      alice.send( rtppacket( { sn: 1, fill: 0xaa } ), a.local.port, "127.0.0.1" )
      carol.send( rtppacket( { sn: 1, fill: 0xcc } ), c.local.port, "127.0.0.1" )
      await wait( 300 )

      const [ atalice, atbob, atcarol ] = got.map( media )
      expect( atalice.map( ( m ) => m[ 12 ] ) ).to.deep.equal( [ 0xcc ] )
      expect( atcarol.map( ( m ) => m[ 12 ] ) ).to.deep.equal( [ 0xaa ] )
      expect( atbob ).to.have.lengthOf( 0 )
      /* each is a single stream on the receiving leg's own SSRC */
      expect( atcarol[ 0 ].readUInt32BE( 8 ) ).to.equal( c.local.ssrc )
      expect( atalice[ 0 ].readUInt32BE( 8 ) ).to.equal( a.local.ssrc )

      await done()
    } )

    it( "routes a keyframe request to the source whose SSRC it names", async function() {
      const { socks, got, legs, done } = await threeway()
      const [ alice, bob, carol ] = socks
      const [ a, b, c ] = legs

      alice.send( rtppacket( { sn: 1, ssrc: 0xa11ce, fill: 0xaa } ), a.local.port, "127.0.0.1" )
      bob.send( rtppacket( { sn: 1, ssrc: 0xb0b, fill: 0xbb } ), b.local.port, "127.0.0.1" )
      await wait( 500 ) /* also clears the join-time PLI rate limit window */

      const atcarol = media( got[ 2 ] )
      const bobsssrc = atcarol.find( ( m ) => 0xbb === m[ 12 ] ).readUInt32BE( 8 )
      const alicessrc = atcarol.find( ( m ) => 0xaa === m[ 12 ] ).readUInt32BE( 8 )
      expect( bobsssrc ).to.not.equal( alicessrc )

      const plis = ( i ) => got[ i ].filter( ispli ).length
      const before = [ plis( 0 ), plis( 1 ) ]

      /* carol asks for a keyframe of bob's stream only, the browser way */
      carol.send( Buffer.concat( [ rrpacket( 0x1 ), plipacket( 0x1, bobsssrc ) ] ), c.local.port, "127.0.0.1" )
      await wait( 300 )

      expect( plis( 1 ) ).to.equal( before[ 1 ] + 1 )
      expect( plis( 0 ) ).to.equal( before[ 0 ] )
      const pli = got[ 1 ].filter( ispli ).pop()
      expect( pli.readUInt32BE( 4 ) ).to.equal( b.local.ssrc )
      expect( pli.readUInt32BE( 8 ) ).to.equal( 0xb0b )

      await done()
    } )
  } )

  it( "audio channels refuse to mix with a relay channel", async function() {

    const closed = []
    const mkclose = () => {
      let done
      closed.push( new Promise( ( r ) => { done = r } ) )
      return ( d ) => { if( "close" === d.action ) done() }
    }

    const relay = await projectrtp.openchannel(
      { "relay": true, "forcelocal": true, "remote": { "address": "127.0.0.1", "port": 20002, "codec": 96 } }, mkclose() )
    const audio = await projectrtp.openchannel(
      { "forcelocal": true, "remote": { "address": "127.0.0.1", "port": 20004, "codec": 0 } }, mkclose() )

    expect( relay.mix( audio ) ).to.be.false
    expect( audio.mix( relay ) ).to.be.false

    relay.close()
    audio.close()
    await Promise.all( closed )
  } )

  it( "relay channel emits close with stats counting relayed packets", async function() {

    const alice = await bindsocket()
    const bob = await bindsocket()

    const { a, b, closed } = await openrelaypair( alice, bob )

    for( let i = 0; 10 > i; i++ )
      alice.send( rtppacket( { sn: i, ts: 3000 * i } ), a.local.port, "127.0.0.1" )

    await new Promise( ( resolve ) => setTimeout( resolve, 300 ) )

    /* camera off: bob sends no video, only RTCP — that is liveness */
    bob.send( rrpacket( 0x1 ), b.local.port, "127.0.0.1" )
    await new Promise( ( resolve ) => setTimeout( resolve, 100 ) )
    expect( b.livestats().in.rtcp ).to.equal( 1 )
    expect( b.livestats().in.accepted ).to.equal( 0 )

    let astats, bstats
    a.em.on( "close", ( d ) => { astats = d.stats } )
    b.em.on( "close", ( d ) => { bstats = d.stats } )

    a.close()
    b.close()
    await Promise.all( closed )

    expect( bstats ).to.be.an( "object" )
    expect( bstats.out.count ).to.be.above( 0 )
    expect( bstats.relay ).to.be.true
    expect( bstats.in.rtcp ).to.equal( 1 )
    expect( bstats.out.dropped ).to.equal( 0 )
    expect( astats.in.accepted ).to.equal( 10 )
    expect( astats.in.decryptfailed ).to.equal( 0 )
    expect( astats.in.prekey ).to.equal( 0 )

    alice.close()
    bob.close()
  } )

  describe( "abuse limits", function() {

    this.timeout( 6000 )

    const wait = ( ms ) => new Promise( ( resolve ) => setTimeout( resolve, ms ) )

    /**
     * Open a relay pair; `alimits` (if defined) is leg A's relaylimits, and
     * A's remote declares `codecs`.
     * @param { object } alice
     * @param { object } bob
     * @param { object } [ alimits ]
     * @param { object } [ codecs ]
     * @returns { Promise< object > }
     */
    async function limitedpair( alice, bob, alimits, codecs ) {
      const { closed, mkclose } = closetracker()
      const aparams = { "relay": true, "forcelocal": true,
        "remote": { "address": "127.0.0.1", "port": alice.address().port, "codec": 96 } }
      if( undefined !== alimits ) aparams.relaylimits = alimits
      if( codecs ) aparams.remote.codecs = codecs
      const a = await projectrtp.openchannel( aparams, mkclose() )
      const b = await projectrtp.openchannel(
        { "relay": true, "forcelocal": true, "remote": { "address": "127.0.0.1", "port": bob.address().port, "codec": 96 } },
        mkclose() )
      expect( a.mix( b ) ).to.be.true
      const done = async () => {
        a.close()
        b.close()
        await Promise.all( closed )
        alice.close()
        bob.close()
      }
      return { a, b, done }
    }

    /**
     * Send `count` packets of `payloadsize` from alice to leg A, one frame
     * (one timestamp) per `perframe` packets, paced `batch` per 5ms.
     * @param { object } alice
     * @param { object } a
     * @param { number } count
     * @param { number } payloadsize
     * @param { number } [ perframe ]
     * @param { number } [ batch ]
     * @returns { Promise< void > }
     */
    async function blast( alice, a, count, payloadsize, perframe = 100, batch = 10 ) {
      for( let i = 0; count > i; i += batch ) {
        for( let j = i; Math.min( count, i + batch ) > j; j++ )
          alice.send( rtppacket( { sn: j, ts: 3000 * Math.floor( j / perframe ), payloadsize } ), a.local.port, "127.0.0.1" )
        await wait( 5 )
      }
    }

    it( "drops and counts a source over its bitrate cap, forwards one within it", async function() {
      const alice = await bindsocket()
      const bob = await bindsocket()
      let received = 0
      bob.on( "message", () => received++ )
      /* 80 kbit/s, 10KB deep: ~8 near-MTU packets at once, then ~1 per 100ms */
      const { a, b, done } = await limitedpair( alice, bob, { "bitrate": 80000, "burst": 10000 } )

      /* within the cap: 5 x 1012 bytes */
      await blast( alice, a, 5, 1000 )
      await wait( 300 )
      expect( received ).to.equal( 5 )
      expect( a.livestats().in.ratedropped ).to.equal( 0 )

      /* 40KB in ~20ms: far over. The bucket refills at 10KB/s, so more
         than 25 drops holds until the blast takes ~500ms */
      received = 0
      await blast( alice, a, 40, 1000 )
      await wait( 300 )
      const s = a.livestats()
      expect( s.relaylimits ).to.deep.equal( { "bitrate": 80000, "burst": 10000, "packetrate": 1000, "framerate": 30, "maxfs": 3600 } )
      expect( s.in.accepted ).to.equal( 45 )
      expect( s.in.ratedropped ).to.be.above( 25 )
      expect( received ).to.equal( 40 - s.in.ratedropped )
      expect( b.livestats().out.count ).to.equal( 5 + received )
      expect( s.in.framedropped ).to.equal( 0 )
      expect( s.in.oversizedropped ).to.equal( 0 )

      let closestats
      a.em.on( "close", ( d ) => { closestats = d.stats } )
      await done()
      expect( closestats.in.ratedropped ).to.equal( s.in.ratedropped )
      expect( closestats.relaylimits ).to.deep.equal( s.relaylimits )
    } )

    it( "applies the default limits when relaylimits is absent", async function() {
      const alice = await bindsocket()
      const bob = await bindsocket()
      let received = 0
      bob.on( "message", () => received++ )
      const { a, done } = await limitedpair( alice, bob, undefined )

      expect( a.livestats().relaylimits ).to.deep.equal(
        { "bitrate": 2000000, "burst": 250000, "packetrate": 1000, "framerate": 30, "maxfs": 3600 } )

      /* ~390KB in ~40ms against a 250KB bucket refilling at 250KB/s. The
         bucket refills while the blast is sent, so the send is kept short:
         more than 30 drops holds until it takes ~390ms, ~10x slower than it
         runs, which a loaded CI runner does not reach */
      await blast( alice, a, 320, 1188, 320, 40 )
      await wait( 500 )
      const s = a.livestats()
      expect( s.in.ratedropped ).to.be.above( 30 )
      expect( received ).to.equal( s.in.accepted - s.in.ratedropped )
      await done()
    } )

    it( "relaylimits { bitrate: 0 } turns the bitrate cap off", async function() {
      const alice = await bindsocket()
      const bob = await bindsocket()
      let received = 0
      bob.on( "message", () => received++ )
      const { a, done } = await limitedpair( alice, bob, { "bitrate": 0 } )

      expect( a.livestats().relaylimits ).to.deep.equal(
        { "bitrate": 0, "burst": 0, "packetrate": 1000, "framerate": 30, "maxfs": 3600 } )

      /* the same ~390KB burst the default cap cuts */
      await blast( alice, a, 320, 1188, 320, 40 )
      await wait( 500 )
      const s = a.livestats()
      expect( s.in.ratedropped ).to.equal( 0 )
      expect( s.in.accepted ).to.be.above( 280 ) /* > 250KB + refill got through */
      expect( received ).to.equal( s.in.accepted )
      await done()
    } )

    it( "a relaylimits fraction that rounds to 0 takes the default, not unlimited", async function() {
      const alice = await bindsocket()
      const bob = await bindsocket()
      const { a, done } = await limitedpair( alice, bob,
        { "bitrate": 0.4, "packetrate": 1e-9, "framerate": 0.49, "maxfs": 2.6, "burst": -3 } )

      expect( a.livestats().relaylimits ).to.deep.equal(
        { "bitrate": 2000000, "burst": 250000, "packetrate": 1000, "framerate": 30, "maxfs": 3 } )
      await done()
    } )

    it( "a huge relaylimits value reads back as the largest exact integer, not negative", async function() {
      const alice = await bindsocket()
      const bob = await bindsocket()
      const { a, done } = await limitedpair( alice, bob, { "bitrate": 1e30, "maxfs": Number.MAX_SAFE_INTEGER } )

      expect( a.livestats().relaylimits ).to.deep.equal(
        { "bitrate": Number.MAX_SAFE_INTEGER, "burst": Math.floor( Number.MAX_SAFE_INTEGER / 8 ), "packetrate": 1000, "framerate": 30, "maxfs": Number.MAX_SAFE_INTEGER } )
      await done()
    } )

    /**
     * A VP8 RTP payload (RFC 7741 descriptor + RFC 6386 keyframe header).
     * @param { number } w
     * @param { number } h
     * @returns { Buffer }
     */
    function vp8keyframe( w, h ) {
      const p = Buffer.alloc( 14 + 200, 0xaa )
      Buffer.from( [ 0x90, 0x80, 0x92, 0x34, 0x50, 0x2a, 0x01, 0x9d, 0x01, 0x2a ] ).copy( p )
      p.writeUInt16LE( w, 10 )
      p.writeUInt16LE( h, 12 )
      return p
    }

    it( "refuses a VP8 stream whose keyframe is over max-fs until one within it", async function() {
      const alice = await bindsocket()
      const bob = await bindsocket()
      const received = []
      bob.on( "message", ( m ) => received.push( Buffer.from( m ) ) )
      const { a, done } = await limitedpair( alice, bob, undefined, { "vp8": 96 } )

      const send = async ( sn, ts, payload ) => {
        const hdr = rtppacket( { sn, ts, payloadsize: 0 } )
        alice.send( Buffer.concat( [ hdr, payload ] ), a.local.port, "127.0.0.1" )
        await wait( 20 )
      }
      const inter = Buffer.from( [ 0x90, 0x80, 0x92, 0x34, 0x31, 0x02, 0x00, 0x11 ] )
      /* 1920x1080 (8160 macroblocks): the keyframe and what follows it are dropped */
      await send( 1, 3000, vp8keyframe( 1920, 1080 ) )
      await send( 2, 6000, inter )
      /* portrait 720x1280 (3600 macroblocks): through, and the stream with it */
      await send( 3, 9000, vp8keyframe( 720, 1280 ) )
      await send( 4, 12000, inter )
      await wait( 200 )

      const s = a.livestats()
      expect( s.in.oversizedropped ).to.equal( 2 )
      expect( received.map( ( m ) => m.readUInt32BE( 4 ) ) ).to.deep.equal( [ 9000, 12000 ] )
      await done()
    } )
  } )
} )
