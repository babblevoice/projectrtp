


const expect = require( "chai" ).expect
const fs = require( "fs" )

const dgram = require( "dgram" )

const projectrtp = require( "../src/build/Release/projectrtp" )

function createwavheader( chunksize = 8000 ) {
  let audioformat = 1 /* 1 for PCM | 3 for IEEE Float | 6 a law | 7 u law | 0xA112 722 | 0xA116 ilbc */
  let fmtchunksize = 16  /* Should be 16 for PCM */
  let numchannel = 1
  let samplerate = 8000
  let bytespersample = 2
  let byterate = samplerate * numchannel * bytespersample
  let samplealignment = numchannel * bytespersample
  let bitdepth = 16

  const wavheader = Buffer.allocUnsafe( 44 )
  wavheader.write( "RIFF", 0 )
  wavheader.writeUInt32LE( chunksize, 4 )
  wavheader.write( "WAVEfmt ", 8 )
  wavheader.writeInt32LE( fmtchunksize, 16 )
  wavheader.writeUInt16LE( audioformat, 20 )
  wavheader.writeInt16LE( numchannel, 22 )
  wavheader.writeInt32LE( samplerate, 24 )
  wavheader.writeInt32LE( byterate, 28 )
  wavheader.writeInt16LE( samplealignment, 32 )
  wavheader.writeInt16LE( bitdepth, 34 )
  wavheader.write( "data", 36 )
  wavheader.writeUInt32LE( chunksize, 40 )

  return wavheader
}


describe( "rtpsound", function() {
  describe( "play with soundsoup", function() {

    it( `call create channel with zero or none exsisting files and check failure`, async function() {

      this.timeout( 800 )
      this.slow( 600 )

      let channel = projectrtp.rtpchannel.create( { "target": { "address": "localhost", "port": 20000, "codec": 0 } } )

      expect( channel.close ).to.be.an( "function" )
      expect( channel.play ).to.be.an( "function" )

      expect( channel.play( { "files": [] } ) ).to.equal( false )
      expect( channel.play( {} ) ).to.equal( false )
      expect( channel.play( { "files": [ { "wav": "doesntexsist.wav" } ] } ) ).to.equal( false )

      await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 200 ) } )
      channel.close()

    } )

    it( `play simple soundsoup and check udp data`, function( done ) {
      /* create our RTP/UDP endpoint */
      const server = dgram.createSocket( "udp4" )
      var receviedpkcount = 0
      var channel
      var thistime = Date.now() /* mS */

      server.on( "message", function( msg, rinfo ) {

        var nowtime = Date.now()
        var difftime = nowtime - thistime
        thistime = nowtime

        if( 0 === receviedpkcount ) {
          /* start up time as well */
          expect( difftime ).to.be.within( 18, 60 )
        } else {
          expect( difftime ).to.be.within( 18, 22 )
        }

        /* This is PCMA encoded data from our flat file */
        expect( msg[ 16 ] ).to.equal( 0x99 )

        receviedpkcount++
        if( receviedpkcount > 20 ) {
          channel.close()
        }

      } )

      this.timeout( 3000 )
      this.slow( 2500 )

      server.bind()
      server.on( "listening", function() {

        let ourport = server.address().port

        channel = projectrtp.rtpchannel.create( { "target": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {

          if( "close" === d.action ) {
            server.close()
            done()
          }
        } )

        expect( channel.play( {
          "files": [
            { "wav": "/tmp/flat.wav" }
          ]
        } ) ).to.be.true

      } )
    } )
  } )



  before( async () => {

    let chunksize = 8000
    let bytespersample = 2 /* 16 bit audio */
    let wavheader = createwavheader( chunksize )
    const zeros = Buffer.alloc( chunksize * bytespersample )
    for( let i = 0; i < chunksize; i++ ) {
      zeros.writeUInt16BE( 300, i * 2 )
    }

    await fs.writeFile( "/tmp/flat.wav", Buffer.concat( [ wavheader, zeros ] ), function() {} )

    projectrtp.run()
  } )

  after( async () => {
    await projectrtp.shutdown()
    await new Promise( ( resolve, reject ) => { fs.unlink( "/tmp/flat.wav", ( err ) => { resolve() } ) } )
  } )
} )
