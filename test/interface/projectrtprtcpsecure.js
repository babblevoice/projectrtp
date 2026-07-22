

const expect = require( "chai" ).expect
const fs = require( "fs" )

const projectrtp = require( "../../index.js" ).projectrtp

/* Standalone safety net, mirroring projectrtprtcp.js: run() is idempotent so a
   second call in the full suite is a no-op. projectrtpserver.js owns teardown. */
before( () => { projectrtp.run() } )

/* Minimal PCM16 mono WAV writer — a self-contained sine source so this file
   doesn't depend on codecchain.js's helpers. */
function writetonewav( path, tonehz = 400, durationseconds = 2.0, samplerate = 8000, amplitude = 0.5 ) {
  const total = Math.floor( samplerate * durationseconds )
  const peak = Math.round( 32767 * amplitude )
  const data = Buffer.alloc( total * 2 )
  const w = 2 * Math.PI * tonehz / samplerate
  for( let i = 0; total > i; i++ ) data.writeInt16LE( Math.round( Math.sin( i * w ) * peak ), i * 2 )

  const header = Buffer.alloc( 44 )
  header.write( "RIFF", 0 )
  header.writeUInt32LE( 36 + data.length, 4 )
  header.write( "WAVE", 8 )
  header.write( "fmt ", 12 )
  header.writeUInt32LE( 16, 16 )
  header.writeUInt16LE( 1, 20 )  /* PCM */
  header.writeUInt16LE( 1, 22 )  /* mono */
  header.writeUInt32LE( samplerate, 24 )
  header.writeUInt32LE( samplerate * 2, 28 )
  header.writeUInt16LE( 2, 32 )  /* block align */
  header.writeUInt16LE( 16, 34 )
  header.write( "data", 36 )
  header.writeUInt32LE( data.length, 40 )

  fs.writeFileSync( path, Buffer.concat( [ header, data ] ) )
}

describe( "rtcp secure (SRTCP)", function() {

  const wavpath = "/tmp/rtcp_srtcp_tone.wav"
  before( () => { writetonewav( wavpath ) } )
  after( () => { try { fs.unlinkSync( wavpath ) } catch( _ ) { /* ignore */ } } )

  it( "protects RTCP as SRTCP over DTLS and folds the peer's decrypted reports", async function() {

    /* Randomised reports: first fires ~1–3 s, RTT needs a second exchange;
       6.5 s below leaves headroom for both directions. */
    this.timeout( 12000 )
    this.slow( 10000 )

    let statsa, statsb
    let resolvea, resolveb
    const closeda = new Promise( ( r ) => { resolvea = r } )
    const closedb = new Promise( ( r ) => { resolveb = r } )

    const chana = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) {
        statsa = d.stats
        resolvea()
      }
    } )
    const chanb = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) {
        statsb = d.stats
        resolveb()
      }
    } )

    /* chana is the DTLS client (active), chanb the server (passive). Each is
       given the other's fingerprint. This is the same secure topology as the
       codecchain.js DTLS-SRTP test, but here we assert the RTCP path. */
    expect( chana.remote( {
      "address": "127.0.0.1",
      "port": chanb.local.port,
      "codec": 0,
      "dtls": { "fingerprint": { "hash": chanb.local.dtls.fingerprint }, "mode": "active" }
    } ) ).to.be.true

    expect( chanb.remote( {
      "address": "127.0.0.1",
      "port": chana.local.port,
      "codec": 0,
      "dtls": { "fingerprint": { "hash": chana.local.dtls.fingerprint }, "mode": "passive" }
    } ) ).to.be.true

    /* chana plays a looping tone (RTP A→B); chanb echoes it back (RTP B→A).
       Both directions carry SRTP media, so both sides latch the other's SSRC
       and both accumulate out_count > 0 — so each emits an SR that carries a
       reception report about the other. */
    expect( chanb.echo() ).to.be.true
    await new Promise( ( r ) => setTimeout( r, 300 ) ) /* settle the handshake */
    expect( chana.play( { "loop": true, "files": [ { "wav": wavpath } ] } ) ).to.be.true

    /* Wait past the first RTCP interval so each side has sent an SR and the
       other has received, authenticated, decrypted and folded it. */
    await new Promise( ( r ) => setTimeout( r, 6500 ) )

    chana.close()
    chanb.close()
    await Promise.all( [ closeda, closedb ] )

    /* out.valid === true means the peer's SR — SRTCP-encrypted on the wire —
       was received, passed auth, decrypted, and its report block about *us*
       folded into RemoteReport. That is the end-to-end proof the SRTCP path
       works both ways. (rttms stays null until the peer echoes one of our SRs,
       which needs two intervals, so we allow null-or-number here.) */
    for( const [ name, s ] of [ [ "A", statsa ], [ "B", statsb ] ] ) {
      expect( s, `${name} close stats` ).to.have.property( "rtcp" )
      expect( s.rtcp.out.valid, `${name} peer report not valid — SRTCP decrypt failed?` ).to.equal( true )
      expect( s.rtcp.in.cumulativelost, `${name} in.cumulativelost` ).to.be.a( "number" )
      expect( s.rtcp.rttms, `${name} rttms` ).to.satisfy( ( v ) => null === v || "number" === typeof v )
    }
  } )
} )
