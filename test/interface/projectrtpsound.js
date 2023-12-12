

//const npl = require( "nodeplotlib" )
// eslint-disable-next-line no-unused-vars
const npl = { plot: ( /** @type {any} */ a ) => {} }

/*
Note, timing in Node doesn't appear that accurate. This requires more work if
we can measure jitter. The pcap traces show 0.01 mS jitter but we are getting 4mS
when playing in node. For now, leave checking of timing.
*/

const expect = require( "chai" ).expect
const fs = require( "fs" )
const dgram = require( "dgram" )
const prtp = require( "../../index" )

function gensignal( hz, datalength, magnitude ) {

  const y = Buffer.alloc( datalength * 2 )

  for( let i = 0; i < datalength; i ++ ) {
    y.writeInt16LE( Math.sin( i * ( Math.PI * 2 * ( 1 / 8000 ) ) * hz ) * magnitude, i * 2 )
  }

  return y
}

/**
 * Limitation of not parsing ccrc.
 * @param { Buffer } packet
 * @return { object }
 */
function parsepk( packet ) {
  return {
    sn: packet.readUInt16BE( 2 ),
    ts: packet.readUInt32BE( 4 ),
    pt: packet.readUInt8( 1 ) & 0x7f,
    ssrc: packet.readUInt32BE( 8 ),
    payload: new Uint8Array( packet.slice( 12 ) )
  }
}

/**
 * 
 * @param { Array< number > } inarray
 * @returns { Int16Array }
 */
function pcmutolinear( inarray ) {
  const out = new Int16Array( inarray.length )
  for( let i = 0; i < inarray.length; i++ ) {
    out[ i ] = prtp.projectrtp.codecx.pcmu2linear16( inarray[ i ] )
  }

  return out
}

/**
 * Limitation of not parsing ccrc.
 * @param { Buffer } packet
 * @return { object }
 */
function parsertppk( packet ) {
  return {
    sn: packet.readUInt16BE( 2 ),
    ts: packet.readUInt32BE( 4 ),
    pt: packet.readUInt8( 1 ) & 0x7f,
    ssrc: packet.readUInt32BE( 8 ),
    payload: new Uint8Array( packet.slice( 12 ) )
  }
}

/**
 * @param { number } samples
 * @returns { Buffer }
 */
function createwavheader( samples = 8000 ) {
  const audioformat = 1 /* 1 for PCM | 3 for IEEE Float | 6 a law | 7 u law | 0xA112 722 | 0xA116 ilbc */
  const fmtchunksize = 16  /* Should be 16 for PCM */
  const numchannel = 1
  const samplerate = 8000
  const bytespersample = 2
  const byterate = samplerate * numchannel * bytespersample
  const samplealignment = numchannel * bytespersample
  const bitdepth = 16

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

  it( "play file with zero or none exsisting files and check failure", async function() {

    this.timeout( 800 )
    this.slow( 600 )

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const channel = await prtp.projectrtp.openchannel( { "remote": { "address": "localhost", "port": 20000, "codec": 0 } }, ( d ) => {
      if( "close" === d.action ) done()
    } )

    expect( channel.close ).to.be.an( "function" )
    expect( channel.play ).to.be.an( "function" )

    expect( channel.play( { "files": [] } ) ).to.equal( false )
    expect( channel.play( {} ) ).to.equal( false )
    expect( channel.play( { "files": [ { "wav": "doesntexsist.wav" } ] } ) ).to.equal( false )

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 200 ) } )
    channel.close()

    await finished

  } )

  it( "play simple soundsoup and check udp data", async function() {
    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    let receviedpkcount = 0
    let receivedcorrectvalue = 0

    /** @type { prtp.channel } */
    let channel

    server.on( "message", function( msg ) {
      /* This is PCMA encoded data from our flat file */
      const pk = parsertppk( msg )
      /* we can also receive silence */
      if( 228 == pk.payload[ 10 ] ) receivedcorrectvalue++
      receviedpkcount++
    } )

    this.timeout( 3000 )
    this.slow( 2000 )

    let done
    const finished = new Promise( ( r ) => { done = r } )

    server.bind()
    server.on( "listening", async function() {

      const ourport = server.address().port

      channel = await prtp.projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {
        if( "close" === d.action ) done()
      } )

      expect( channel.play( {
        "files": [
          { "wav": "/tmp/flat.wav" }
        ]
      } ) ).to.be.true

    } )

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 1100 ) } )
    if( channel ) channel.close()
    server.close()

    await finished

    expect( receviedpkcount ).to.be.within( 48, 60 )
    expect( receivedcorrectvalue ).to.be.within( 48, 60 )
  } )

  it( "loop in soundsoup and check udp data", function( done ) {

    this.timeout( 6000 )
    this.slow( 5000 )

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    let receviedpkcount = 0
    let channel

    server.on( "message", function( msg ) {

      /* This is PCMA encoded data from our flat file */
      expect( msg[ 16 ] ).to.equal( 0x99 )
      receviedpkcount++
      /*
      flat.wav has 1 S of audio (8000 samples). 160 per packet compressed = 320 PCM.
      200 packets is 64000 samples so this must have looped to work.
      */
      if( 220 < receviedpkcount ) {
        channel.close()
      }
    } )

    server.bind()
    server.on( "listening", async function() {

      const ourport = server.address().port

      channel = await prtp.projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {

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

  /* I have also used this test to play with file codec conversion to check it is working properly */

  it( "loop in soundsoup file and check udp data", function( done ) {

    this.timeout( 6000 )
    this.slow( 5000 )

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    let receviedpkcount = 0
    let channel

    let receivedpcmu = []
    server.on( "message", function( msg ) {
      receivedpcmu = [ ...receivedpcmu,  ...Array.from( pcmutolinear( parsepk( msg ).payload ) ) ]

      /* This is PCMA encoded data from our flat file */
      //expect( msg[ 16 ] ).to.equal( 0x99 )

      receviedpkcount++
      /*
      flat.wav has 1 S of audio (8000 samples). 160 per packet compressed = 320 PCM.
      200 packets is 64000 samples so this must have looped to work.
      */
      if( 220 < receviedpkcount ) {
        channel.close()
      }

    } )

    server.bind()
    server.on( "listening", async function() {

      const ourport = server.address().port

      channel = await prtp.projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {

        if( "close" === d.action ) {
          server.close()

          npl.plot( [ {
            y: receivedpcmu,
            type: "scatter"
          } ] )


          done()
        }
      } )

      expect( channel.play( {
        "files": [
          { "loop": true, "wav": "/tmp/440sine.wav" }
        ]
      } ) ).to.be.true

    } )
  } )

  it( "loop soup twice in soundsoup file and check udp data", async function() {

    this.timeout( 4000 )
    this.slow( 3000 )

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    let receviedpkcount = 0

    /** @type { prtp.channel } */
    let channel

    let done
    const finished = new Promise( ( r ) => { done = r } )

    server.on( "message", function( msg ) {

      receviedpkcount++

      /* This is PCMA encoded data from our flat file */
      expect( msg[ 17 ] ).to.equal( 153 /* 0x99 */ )
      expect( msg[ 150 ] ).to.equal( 153 /* 0x99 */ )
    } )

    server.bind()
    server.on( "listening", async function() {

      const ourport = server.address().port

      channel = await prtp.projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {
        if( "close" === d.action ) done()
      } )

      expect( channel.play( { "loop": 2, "files": [ { "wav": "/tmp/flat.wav" } ] } ) ).to.be.true

    } )

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 2200 ) } )
    channel.close()
    server.close()

    await finished
    /*
      8000 samples looped twice. 100 packets (50 packets/S).
    */

    expect( receviedpkcount ).to.be.within( 97, 100 )
  } )

  it( "slightly more complex soundsoup file and check udp data", async function() {

    this.timeout( 10000 )
    this.slow( 9500 )

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    let receviedpkcount = 0

    /** @type { prtp.channel } */
    let channel

    let done
    const finished = new Promise( ( r ) => { done = r } )

    server.on( "message", function( msg ) {

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

      const ourport = server.address().port

      channel = await prtp.projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {
        if( "close" === d.action ) done()
      } )

      expect( channel.play( { "loop": 2, "files": [
        { "wav": "/tmp/flat.wav", "loop": 2 },
        { "wav": "/tmp/flat2.wav" },
        { "wav": "/tmp/flat.wav" },
        { "wav": "/tmp/flat3.wav", "start": 40, "stop": 60 }, /* should be 2 packets */
      ] } ) ).to.be.true
    } )

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 9000 ) } )
    channel.close()
    server.close()

    await finished

    /*
      8000 samples looped twice with 3 sections to play. 400 packets (50 packets/S).
    */
    expect( receviedpkcount ).to.be.within( 390, 405 )
  } )

  before( async () => {

    const samples = 8000
    const bytespersample = 2 /* 16 bit audio */
    const wavheader = createwavheader( samples )
    const values = Buffer.alloc( samples * bytespersample )
    for( let i = 0; i < samples; i++ ) {
      values.writeUInt16LE( 300, i * 2 )
    }
    /* Put a marker at the start of the file */
    values.writeUInt16LE( 1000, 50 )

    await fs.writeFile( "/tmp/flat.wav", Buffer.concat( [ wavheader, values ] ), function() {} )

    for( let i = 0; i < samples; i++ ) {
      values.writeUInt16LE( 400, i * 2 )
    }

    await fs.writeFile( "/tmp/flat2.wav", Buffer.concat( [ wavheader, values ] ), function() {} )

    for( let i = 0; i < samples; i++ ) {
      values.writeUInt16LE( 0, i * 2 )
    }


    for( let i = 0; 320 > i; i++ ) {
      values.writeUInt16LE( 500, 640 + ( i * 2 ) )
    }

    await fs.writeFile( "/tmp/flat3.wav", Buffer.concat( [ wavheader, values ] ), function() {} )

    const sig = gensignal( 440, 8000, 1000 )
    await fs.writeFile( "/tmp/440sine.wav", Buffer.concat( [ wavheader, sig ] ), function() {} )

    const uint16Array = []
    for( let i = 0; i < sig.length; i += 2 ) {
      uint16Array.push( sig.readInt16LE( i ) )
    }

  } )

  after( async () => {
    await new Promise( ( resolve ) => { fs.unlink( "/tmp/flat.wav", () => { resolve() } ) } )
    await new Promise( ( resolve ) => { fs.unlink( "/tmp/flat2.wav", () => { resolve() } ) } )
    await new Promise( ( resolve ) => { fs.unlink( "/tmp/flat3.wav", () => { resolve() } ) } )
    await new Promise( ( resolve ) => { fs.unlink( "/tmp/440sine.wav", () => { resolve() } ) } )
  } )
} )
