

const expect = require( "chai" ).expect
const fs = require( "fs" )
const fspromises = fs.promises

const dgram = require( "dgram" )

const projectrtp = require( "../src/build/Release/projectrtp" )

function sendpk( sn, sendtime, dstport, server, ssrc = 25, pklength = 172 ) {

  setTimeout( () => {

    let payload = Buffer.alloc( pklength - 12 ).fill( projectrtp.codecx.linear162pcmu( sn ) & 0xff )
    let subheader = Buffer.alloc( 10 )

    let ts = sn * 160

    subheader.writeUInt16BE( ( sn + 100 ) % ( 2**16 ) /* just some offset */ )
    subheader.writeUInt32BE( ts, 2 )
    subheader.writeUInt32BE( ssrc, 6 )

    let rtppacket = Buffer.concat( [
      Buffer.from( [ 0x80, 0x00 ] ),
      subheader,
      payload ] )

    server.send( rtppacket, dstport, "localhost" )
  }, sendtime * 20 )
}


describe( "rtpsound", function() {
  describe( "record", function() {

    /* not finished */
    it( `record to file`, async function() {
      /* create our RTP/UDP endpoint */
      const server = dgram.createSocket( "udp4" )
      var receviedpkcount = 0
      var channel

      server.on( "message", function( msg, rinfo ) {
        receviedpkcount++
      } )

      this.timeout( 1500 )
      this.slow( 1200 )

      server.bind()
      server.on( "listening", function() {

        channel = projectrtp.rtpchannel.create( { "target": { "address": "localhost", "port": server.address().port, "codec": 0 } }, function( d ) {
          if( "close" === d.action ) {
            server.close()
            done()
          }
        } )

        expect( channel.record( {
          "file": "/tmp/ourrecording.wav"
        } ) ).to.be.true

        /* something to record */
        expect( channel.echo() ).to.be.true

        for( let i = 0;  i < 50; i ++ ) {
          sendpk( i, i, channel.port, server )
        }
      } )

      await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 1100 ) } )
      channel.close()

      /* Now test the file */
      let wavinfo = projectrtp.soundfile.info( "/tmp/ourrecording.wav" )
      expect( wavinfo.audioformat ).to.equal( 1 )
      expect( wavinfo.channelcount ).to.equal( 2 )
      expect( wavinfo.samplerate ).to.equal( 8000 )
      expect( wavinfo.byterate ).to.equal( 32000 )
      expect( wavinfo.bitdepth ).to.equal( 16 )
      expect( wavinfo.chunksize ).to.equal( 28240 )
      expect( wavinfo.fmtchunksize ).to.equal( 16 )
      expect( wavinfo.subchunksize ).to.equal( 28204 )

      let ourfile = await fspromises.open( "/tmp/ourrecording.wav", "r" )
      const buffer = Buffer.alloc( 28204 )

      /* our payload is the sn - but then we pcma decode to store in the file */
      ourfile.read( buffer, 0, 28204, 45 )
      await ourfile.close()

      /*
      16 bit 2 channels
      160 samples per packet (pcmu to 16l) * 50 per second = 32000
      sn = 4 it pcmu reduces
      160 * 2 * 2 * sn = 2560
      */
      for( let sn = 0; sn < 42; sn++ ){
        expect( buffer.readInt16BE( 160 * 2 * 2 * sn ) )
          .to.equal( projectrtp.codecx.pcmu2linear16( projectrtp.codecx.linear162pcmu( sn ) ) )
      }

    } )
  } )

  before( async () => {
    projectrtp.run()
  } )

  after( async () => {
    await projectrtp.shutdown()
    await new Promise( ( resolve, reject ) => { fs.unlink( "/tmp/ourrecording.wav", ( err ) => { resolve() } ) } )
  } )
} )
