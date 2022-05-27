
/*
Note, timing in Node doesn't appear that accurate. This requires more work if
we can measure jitter. The pcap traces show 0.01 mS jitter but we are getting 4mS
when playing in node. For now, leave checking of timing.
*/

const expect = require( "chai" ).expect
const fs = require( "fs" )
const dgram = require( "dgram" )
const projectrtp = require( "../../index.js" ).projectrtp


/**
@summary wav file header
@memberof projectrtp
@hideconstructor
@class
*/
function wavinfo() {
  /** @member {number} */
  this.audioformat
  /** @member {number} */
  this.channelcount
  /** @member {number} */
  this.samplerate
  /** @member {number} */
  this.byterate
  /** @member {number} */
  this.bitdepth
  /** @member {number} */
  this.chunksize
  /** @member {number} */
  this.fmtchunksize
  /** @member {number} */
  this.subchunksize
}

/**
@summary Sound (wav) utils
@hideconstructor
*/

class soundfile {
  /**
  @summary Loads wav header and returns info
  @param {string} filename - path and filename
  @returns {wavinfo}
  */
  info( filename ){}
}

function createwavheader( samples = 8000 ) {
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
  wavheader.writeUInt32LE( ( samples * numchannel * bytespersample ) + 36, 4 )
  wavheader.write( "WAVEfmt ", 8 )
  wavheader.writeInt32LE( fmtchunksize, 16 )
  wavheader.writeUInt16LE( audioformat, 20 )
  wavheader.writeInt16LE( numchannel, 22 )
  wavheader.writeInt32LE( samplerate, 24 )
  wavheader.writeInt32LE( byterate, 28 )
  wavheader.writeInt16LE( samplealignment, 32 )
  wavheader.writeInt16LE( bitdepth, 34 )
  wavheader.write( "data", 36 )
  wavheader.writeUInt32LE( samples * numchannel * bytespersample, 40 )

  return wavheader
}


describe( "rtpsound", function() {
  //describe( "play with soundsoup", function() {

  it( `play file with zero or none exsisting files and check failure`, async function() {

    this.timeout( 800 )
    this.slow( 600 )

    let done
    let finished = new Promise( ( r ) => { done = r } )

    let channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": 20000, "codec": 0 } }, ( d ) => {
      if( "close" === d.action ) done()
    } )

    expect( channel.close ).to.be.an( "function" )
    expect( channel.play ).to.be.an( "function" )

    expect( channel.play( { "files": [] } ) ).to.equal( false )
    expect( channel.play( {} ) ).to.equal( false )
    expect( channel.play( { "files": [ { "wav": "doesntexsist.wav" } ] } ) ).to.equal( false )

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 200 ) } )
    channel.close()

    await finished

  } )

  it( `play simple soundsoup and check udp data`, async function() {
    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    var receviedpkcount = 0
    var channel

    server.on( "message", function( msg, rinfo ) {
      /* This is PCMA encoded data from our flat file */
      expect( msg[ 16 ] ).to.equal( 0x99 )
      receviedpkcount++
    } )

    this.timeout( 3000 )
    this.slow( 2000 )

    let done
    let finished = new Promise( ( r ) => { done = r } )

    server.bind()
    server.on( "listening", async function() {

      let ourport = server.address().port

      channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {
        if( "close" === d.action ) done()
      } )

      expect( channel.play( {
        "files": [
          { "wav": "/tmp/flat.wav" }
        ]
      } ) ).to.be.true

    } )

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 1100 ) } )
    channel.close()
    server.close()

    await finished

    expect( receviedpkcount ).to.be.within( 48, 50 )
  } )

  it( `loop in soundsoup and check udp data`, function( done ) {

    this.timeout( 6000 )
    this.slow( 5000 )

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    var receviedpkcount = 0
    var channel

    server.on( "message", function( msg, rinfo ) {

      /* This is PCMA encoded data from our flat file */
      expect( msg[ 16 ] ).to.equal( 0x99 )
      receviedpkcount++
      /*
      flat.wav has 1 S of audio (8000 samples). 160 per packet compressed = 320 PCM.
      200 packets is 64000 samples so this must have looped to work.
      */
      if( receviedpkcount > 220 ) {
        channel.close()
      }
    } )

    server.bind()
    server.on( "listening", async function() {

      let ourport = server.address().port

      channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {

        if( "close" === d.action ) {
          server.close()
          done()
        }
      } )

      expect( channel.play( {
        "loop": true,
        "files": [
          { "wav": "/tmp/flat.wav" }
        ]
      } ) ).to.be.true

    } )
  } )

  it( `loop in soundsoup file and check udp data`, function( done ) {

    this.timeout( 6000 )
    this.slow( 5000 )

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    var receviedpkcount = 0
    var channel

    server.on( "message", function( msg, rinfo ) {

      /* This is PCMA encoded data from our flat file */
      expect( msg[ 16 ] ).to.equal( 0x99 )

      receviedpkcount++
      /*
      flat.wav has 1 S of audio (8000 samples). 160 per packet compressed = 320 PCM.
      200 packets is 64000 samples so this must have looped to work.
      */
      if( receviedpkcount > 220 ) {
        channel.close()
      }

    } )

    server.bind()
    server.on( "listening", async function() {

      let ourport = server.address().port

      channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {

        if( "close" === d.action ) {
          server.close()
          done()
        }
      } )

      expect( channel.play( {
        "files": [
          { "loop": true, "wav": "/tmp/flat.wav" }
        ]
      } ) ).to.be.true

    } )
  } )

  it( `loop soup twice in soundsoup file and check udp data`, async function() {

    this.timeout( 4000 )
    this.slow( 3000 )

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    var receviedpkcount = 0
    var channel

    let done
    let finished = new Promise( ( r ) => { done = r } )

    server.on( "message", function( msg, rinfo ) {

      receviedpkcount++

      /* This is PCMA encoded data from our flat file */
      expect( msg[ 17 ] ).to.equal( 153 /* 0x99 */ )
      expect( msg[ 150 ] ).to.equal( 153 /* 0x99 */ )
    } )

    server.bind()
    server.on( "listening", async function() {

      let ourport = server.address().port

      channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {
        if( "close" === d.action ) done()
      } )

      expect( channel.play( { "loop": 2, "files": [ { "wav": "/tmp/flat.wav" } ] } ) ).to.be.true

    } )

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 2200 ) } )
    channel.close()
    server.close()

    await finished
    /*
      8000 samples looped twice. 100 packets (50 packets/S).
    */

    expect( receviedpkcount ).to.be.within( 97, 100 )
  } )

  it( `slightly more complex soundsoup file and check udp data`, async function() {

    this.timeout( 10000 )
    this.slow( 9500 )

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    var receviedpkcount = 0
    var channel

    let done
    let finished = new Promise( ( r ) => { done = r } )

    server.on( "message", function( msg, rinfo ) {

      receviedpkcount++

      /* This is PCMA encoded data from our soundsoup:
      { "loop": 2, "files": [
        { "wav": "/tmp/flat.wav", "loop": 2 },
        { "wav": "/tmp/flat2.wav" },
        { "wav": "/tmp/flat.wav" },
        { "wav": "/tmp/flat3.wav", "start": 40, "stop": 60 },  should be 2 packets
      ] }
      */
      if( 75 == receviedpkcount ) {
        /* flat.wav */
        expect( msg[ 17 ] ).to.equal( 153 /* 0x99 */ )
        expect( msg[ 150 ] ).to.equal( 153 /* 0x99 */ )
      } else if( 125 == receviedpkcount ) {
        /* flat2.wav */
        expect( msg[ 17 ] ).to.equal( 3 )
        expect( msg[ 150 ] ).to.equal( 3 )
      } else if( 175 == receviedpkcount ) {
        /* flat.wav */
        expect( msg[ 17 ] ).to.equal( 153 /* 0x99 */ )
        expect( msg[ 150 ] ).to.equal( 153 /* 0x99 */ )
      } else if( 250 == receviedpkcount ) {
        /* flat.wav */
        expect( msg[ 17 ] ).to.equal( 153 )
        expect( msg[ 150 ] ).to.equal( 153 )
      } else if( 325 == receviedpkcount ) {
        /* flat2.wav */
        expect( msg[ 17 ] ).to.equal( 3 )
        expect( msg[ 150 ] ).to.equal( 3 )
      } else if( 375 == receviedpkcount ) {
        /* flat.wav */
        expect( msg[ 17 ] ).to.equal( 153 )
        expect( msg[ 150 ] ).to.equal( 153 )
      } else if( 403 == receviedpkcount ) {
        /* flat3.wav */
        expect( msg[ 17 ] ).to.equal( 54 )
        expect( msg[ 150 ] ).to.equal( 54 )
      }
    } )

    server.bind()
    server.on( "listening", async function() {

      let ourport = server.address().port

      channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {
        if( "close" === d.action ) done()
      } )

      expect( channel.play( { "loop": 2, "files": [
        { "wav": "/tmp/flat.wav", "loop": 2 },
        { "wav": "/tmp/flat2.wav" },
        { "wav": "/tmp/flat.wav" },
        { "wav": "/tmp/flat3.wav", "start": 40, "stop": 60 }, /* should be 2 packets */
      ] } ) ).to.be.true
    } )

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 9000 ) } )
    channel.close()
    server.close()

    await finished

    /*
      8000 samples looped twice with 3 sections to play. 400 packets (50 packets/S).
    */
    expect( receviedpkcount ).to.be.within( 390, 405 )
  } )

  before( async () => {

    let samples = 8000
    let bytespersample = 2 /* 16 bit audio */
    let wavheader = createwavheader( samples )
    const values = Buffer.alloc( samples * bytespersample )
    for( let i = 0; i < samples; i++ ) {
      values.writeUInt16BE( 300, i * 2 )
    }
    /* Put a marker at the start of the file */
    values.writeUInt16BE( 1000, 50 )

    await fs.writeFile( "/tmp/flat.wav", Buffer.concat( [ wavheader, values ] ), function() {} )

    for( let i = 0; i < samples; i++ ) {
      values.writeUInt16BE( 400, i * 2 )
    }

    await fs.writeFile( "/tmp/flat2.wav", Buffer.concat( [ wavheader, values ] ), function() {} )

    for( let i = 0; i < samples; i++ ) {
      values.writeUInt16BE( 0, i * 2 )
    }


    for( let i = 0; i < 320; i++ ) {
      values.writeUInt16BE( 500, 640 + ( i * 2 ) )
    }

    await fs.writeFile( "/tmp/flat3.wav", Buffer.concat( [ wavheader, values ] ), function() {} )

  } )

  after( async () => {
    await new Promise( ( resolve, reject ) => { fs.unlink( "/tmp/flat.wav", ( err ) => { resolve() } ) } )
    await new Promise( ( resolve, reject ) => { fs.unlink( "/tmp/flat2.wav", ( err ) => { resolve() } ) } )
    await new Promise( ( resolve, reject ) => { fs.unlink( "/tmp/flat3.wav", ( err ) => { resolve() } ) } )
  } )
} )
