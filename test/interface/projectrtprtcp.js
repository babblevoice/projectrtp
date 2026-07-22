

const expect = require( "chai" ).expect
const dgram = require( "dgram" )

const projectrtp = require( "../../index.js" ).projectrtp

/* Standalone safety net: in the full suite projectrtpserver.js's root before()
   already calls run(); run() is idempotent so a second call here is a no-op.
   No after()/shutdown here — projectrtpserver.js owns teardown, and mocha's
   --exit reaps the process for a standalone run. */
before( () => { projectrtp.run() } )

/* Send one PCMU RTP packet (seq = sn+100, ts = sn*160, ssrc = 25) from `peer`
   to the channel's local port, scheduled at sn*20ms — mirrors the helper used
   across the other interface suites. */
function sendpk( sn, dstport, peer ) {
  return setTimeout( () => {
    const payload = Buffer.alloc( 160 ).fill( sn & 0xff )
    const header = Buffer.alloc( 12 )
    header[ 0 ] = 0x80
    header[ 1 ] = 0x00 /* PT 0 = PCMU */
    header.writeUInt16BE( ( sn + 100 ) % ( 2 ** 16 ), 2 )
    header.writeUInt32BE( sn * 160, 4 )
    header.writeUInt32BE( 25, 8 )
    peer.send( Buffer.concat( [ header, payload ] ), dstport, "127.0.0.1" )
  }, sn * 20 )
}

/* Walk a compound RTCP datagram into { pt, offset } records. length is the
   RFC 3550 sub-packet size in 32-bit words minus one. */
function walkrtcp( buf ) {
  const items = []
  let off = 0
  while( off + 4 <= buf.length ) {
    const pt = buf[ off + 1 ]
    const words = buf.readUInt16BE( off + 2 )
    items.push( { pt, off } )
    off += ( words + 1 ) * 4
  }
  return items
}

/* Build a receiver report (PT 201) with a single report block about `aboutssrc`
   — the stream the channel sends — so the channel's rtcp_loop folds it into its
   RemoteReport. LSR/DLSR left 0 so no RTT is derived (rttms stays null). */
function buildrr( senderssrc, aboutssrc, fraction, cumulative, jitter ) {
  const b = Buffer.alloc( 32 )
  b[ 0 ] = 0x80 | 0x01 /* V=2, RC=1 */
  b[ 1 ] = 201
  b.writeUInt16BE( 7, 2 ) /* (32 bytes / 4) - 1 */
  b.writeUInt32BE( senderssrc >>> 0, 4 )
  b.writeUInt32BE( aboutssrc >>> 0, 8 )
  b[ 12 ] = fraction
  b[ 13 ] = ( cumulative >> 16 ) & 0xff
  b[ 14 ] = ( cumulative >> 8 ) & 0xff
  b[ 15 ] = cumulative & 0xff
  b.writeUInt32BE( 0, 16 ) /* ext highest seq */
  b.writeUInt32BE( jitter, 20 )
  b.writeUInt32BE( 0, 24 ) /* LSR */
  b.writeUInt32BE( 0, 28 ) /* DLSR */
  return b
}

describe( "rtcp", function() {

  it( "emits SR + SDES on P+1 and reflects a peer RR in the close stats", async function() {

    /* Randomised first report fires ~1–3 s (half a randomised interval); headroom. */
    this.timeout( 9000 )
    this.slow( 8000 )

    const rtp = dgram.createSocket( "udp4" )
    const rtcp = dgram.createSocket( "udp4" )
    rtp.on( "message", () => {} ) /* drain echoed audio */

    /* Bind the RTP peer, then bind the RTCP peer at its port + 1 — the address
       the channel derives for symmetric RTCP without mux. */
    await new Promise( ( res ) => rtp.bind( res ) )
    const peerport = rtp.address().port
    await new Promise( ( res, rej ) =>
      rtcp.bind( peerport + 1, ( e ) => ( e ? rej( e ) : res() ) ) )

    let closestats
    let resolveclose
    const closed = new Promise( ( res ) => { resolveclose = res } )

    const gotsr = new Promise( ( res ) =>
      rtcp.once( "message", ( m ) => res( m ) ) )

    const channel = await projectrtp.openchannel(
      { "remote": { "address": "127.0.0.1", "port": peerport, "codec": 0 } },
      function( d ) {
        if( "close" === d.action ) {
          closestats = d.stats
          resolveclose()
        }
      } )

    /* Echo so out_count > 0 → the periodic report is an SR (not an RR), and
       feed 50 packets up front so the channel latches the remote address and
       has a reception report to send about the peer. */
    expect( channel.echo() ).to.be.true
    for( let i = 0; 50 > i; i++ ) sendpk( i, channel.local.port, rtp )

    /* First compound the channel sends us. */
    const sr = await gotsr

    expect( sr.length ).to.be.at.least( 28 )
    expect( sr[ 0 ] >> 6 ).to.equal( 2 ) /* RTP version 2 */
    const items = walkrtcp( sr )
    expect( items[ 0 ].pt ).to.equal( 200 ) /* first sub-packet is an SR */
    expect( items.some( ( it ) => 202 === it.pt ) ).to.be.true /* SDES present */
    /* SR sender SSRC == the channel's SSRC. */
    expect( sr.readUInt32BE( 4 ) ).to.equal( channel.local.ssrc >>> 0 )

    /* Send a crafted RR about the channel's stream; the channel should fold it
       into RemoteReport and surface it on close. */
    rtcp.send(
      buildrr( 25, channel.local.ssrc, 25, 12, 40 ),
      channel.local.port + 1, "127.0.0.1" )

    /* Give the rtcp_loop a moment to process, then close. */
    await new Promise( ( r ) => setTimeout( r, 200 ) )
    channel.close()
    await closed

    rtp.close()
    rtcp.close()

    /* The Close event carries the RTCP summary. */
    expect( closestats ).to.have.property( "rtcp" )
    const r = closestats.rtcp
    /* Our reception of the peer (50 clean packets → ~no loss). */
    expect( r.in.valid ).to.equal( true )
    expect( r.in.cumulativelost ).to.be.a( "number" )
    expect( r.in.jitter ).to.be.a( "number" )
    /* The peer's reported reception of us — the crafted RR values. */
    expect( r.out.valid ).to.equal( true )
    expect( r.out.fractionlost ).to.equal( 25 )
    expect( r.out.cumulativelost ).to.equal( 12 )
    expect( r.out.jitter ).to.equal( 40 )
    /* LSR was 0 in the RR, so no RTT could be derived. */
    expect( r.rttms ).to.equal( null )
  } )

  it( "carries RTCP over the RTP port when rtcp-mux is negotiated (RFC 5761)", async function() {

    /* Randomised first report fires ~1-3s; keep the P+1-test headroom. */
    this.timeout( 9000 )
    this.slow( 8000 )

    const rtp = dgram.createSocket( "udp4" )
    const rtcp = dgram.createSocket( "udp4" ) /* the P+1 port — must stay silent under mux */

    /* Resolve on the first RTCP compound seen *on the RTP port* — demux by the
       RTCP packet-type byte (200..=204), ignoring the echoed PCMU audio. */
    let resolvesr
    const gotsr = new Promise( ( res ) => { resolvesr = res } )
    rtp.on( "message", ( m ) => {
      if( 2 <= m.length && 200 <= m[ 1 ] && 204 >= m[ 1 ] ) resolvesr( m )
    } )

    let p1count = 0
    rtcp.on( "message", () => { p1count++ } )

    await new Promise( ( res ) => rtp.bind( res ) )
    const peerport = rtp.address().port
    await new Promise( ( res, rej ) =>
      rtcp.bind( peerport + 1, ( e ) => ( e ? rej( e ) : res() ) ) )

    let closestats
    let resolveclose
    const closed = new Promise( ( res ) => { resolveclose = res } )

    const channel = await projectrtp.openchannel(
      { "remote": { "address": "127.0.0.1", "port": peerport, "codec": 0, "rtcpmux": true } },
      function( d ) {
        if( "close" === d.action ) {
          closestats = d.stats
          resolveclose()
        }
      } )

    expect( channel.echo() ).to.be.true
    for( let i = 0; 50 > i; i++ ) sendpk( i, channel.local.port, rtp )

    /* The channel's first RTCP compound — arriving on the RTP port, not P+1. */
    const sr = await gotsr

    expect( sr[ 0 ] >> 6 ).to.equal( 2 ) /* RTP version 2 */
    const items = walkrtcp( sr )
    expect( items[ 0 ].pt ).to.equal( 200 ) /* first sub-packet is an SR */
    expect( items.some( ( it ) => 202 === it.pt ) ).to.be.true /* SDES present */
    expect( sr.readUInt32BE( 4 ) ).to.equal( channel.local.ssrc >>> 0 )

    /* Feed a crafted RR back over the *same* RTP port (mux) about our stream;
       the recv_loop must demux it to the RTCP path and fold it. */
    rtp.send(
      buildrr( 25, channel.local.ssrc, 25, 12, 40 ),
      channel.local.port, "127.0.0.1" )

    await new Promise( ( r ) => setTimeout( r, 200 ) )
    channel.close()
    await closed

    rtp.close()
    rtcp.close()

    /* The muxed RR was folded and surfaced in the close stats. */
    expect( closestats ).to.have.property( "rtcp" )
    expect( closestats.rtcp.out.valid ).to.equal( true )
    expect( closestats.rtcp.out.fractionlost ).to.equal( 25 )
    expect( closestats.rtcp.out.cumulativelost ).to.equal( 12 )
    expect( closestats.rtcp.out.jitter ).to.equal( 40 )

    /* Nothing should ever land on the separate P+1 control port under mux. */
    expect( p1count ).to.equal( 0 )
  } )

  it( "sends an RTCP BYE (PT 203) on channel close", async function() {

    this.timeout( 4000 )
    this.slow( 3000 )

    const rtp = dgram.createSocket( "udp4" )
    const rtcp = dgram.createSocket( "udp4" )
    rtp.on( "message", () => {} ) /* drain echoed audio */

    await new Promise( ( res ) => rtp.bind( res ) )
    const peerport = rtp.address().port
    await new Promise( ( res, rej ) =>
      rtcp.bind( peerport + 1, ( e ) => ( e ? rej( e ) : res() ) ) )

    /* Resolve on the first compound that carries a BYE sub-packet. Closing
       early (before the first periodic report) means the BYE is the only
       datagram, but the filter is robust either way. */
    let resolvebye
    const gotbye = new Promise( ( res ) => { resolvebye = res } )
    rtcp.on( "message", ( m ) => {
      if( walkrtcp( m ).some( ( it ) => 203 === it.pt ) ) resolvebye( m )
    } )

    const channel = await projectrtp.openchannel(
      { "remote": { "address": "127.0.0.1", "port": peerport, "codec": 0 } },
      function() {} )

    /* Feed a few packets so the channel latches the remote address, then close
       — the BYE is emitted on the close path. */
    expect( channel.echo() ).to.be.true
    for( let i = 0; 10 > i; i++ ) sendpk( i, channel.local.port, rtp )
    await new Promise( ( r ) => setTimeout( r, 300 ) )
    channel.close()

    const bye = await gotbye
    expect( walkrtcp( bye ).some( ( it ) => 203 === it.pt ) ).to.be.true

    rtp.close()
    rtcp.close()
  } )

  it( "populates in.skip and lowers MOS when inbound packets are lost", function( done ) {

    const peer = dgram.createSocket( "udp4" )
    peer.on( "message", () => {} )

    this.timeout( 3000 )
    this.slow( 2500 )

    peer.bind()
    peer.on( "listening", async function() {

      const peerport = peer.address().port

      const channel = await projectrtp.openchannel(
        { "remote": { "address": "127.0.0.1", "port": peerport, "codec": 0 } },
        function( d ) {
          if( "close" === d.action ) {
            /* 10 of 50 inbound packets are dropped as sequence gaps, so RFC 3550
               receiver accounting now records real loss. Before the fix in_skip
               was never incremented and MOS was pinned at 4.5. */
            expect( d.stats.in.skip ).to.be.above( 0 )
            expect( d.stats.in.mos ).to.be.below( 4.4 )
            peer.close()
            done()
          }
        } )

      expect( channel.echo() ).to.be.true

      /* Drop 10 interior packets; keep seq 0 and 49 so the full range 100..149
         is present and every gap counts as a loss. */
      const drop = { 5: 0, 10: 0, 15: 0, 20: 0, 25: 0, 30: 0, 35: 0, 40: 0, 44: 0, 47: 0 }
      for( let i = 0; 50 > i; i++ ) {
        if( i in drop ) continue
        sendpk( i, channel.local.port, peer )
      }

      setTimeout( () => channel.close(), 1500 )
    } )
  } )
} )
