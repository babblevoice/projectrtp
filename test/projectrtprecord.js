

const expect = require( "chai" ).expect
const fs = require( "fs" )
const fspromises = fs.promises

const dgram = require( "dgram" )

let projectrtp
if( "debug" === process.env.build ) {
  projectrtp = require( "../src/build/Debug/projectrtp" )
} else {
  projectrtp = require( "../src/build/Release/projectrtp" )
}

/* use nodeplotlib to display audio data */
const showplots = false

if( showplots ) {
  const plot = require( "nodeplotlib" )
}

function int16bebuffer2array( inbuffer ) {
  let r = []
  for( let i = 0; i < inbuffer.length; i = i + 2 ) {
    r.push( inbuffer.readInt16BE( i ) )
  }
  return r
}

function pcmubuffer2array( inbuffer ) {
  let r = []
  for( let i = 0; i < inbuffer.length; i++ ) {
    r.push( projectrtp.codecx.pcmu2linear16( inbuffer[ i ] ) )
  }
  return r
}

function genpcmutone( durationseconds = 0.25, tonehz = 100, samplerate = 16000, amp = 15000 ) {

  const tonebuffer = Buffer.alloc( samplerate*durationseconds, projectrtp.codecx.linear162pcmu( 0 ) )

  for( let i = 0; i < tonebuffer.length; i++ ) {
    let val = Math.sin( ( i / samplerate ) * Math.PI * tonehz ) * amp
    tonebuffer[ i ] = projectrtp.codecx.linear162pcmu( val )
  }

  return tonebuffer
}

function sendpk( sn, sendtime, dstport, server, data = undefined ) {

  let ssrc = 25
  let pklength = 172

  return setTimeout( () => {

    let payload
    if( undefined != data ) {
      let start = sn * 160
      let end = start + 160
      payload = data.subarray( start, end )
    } else {
      payload = Buffer.alloc( pklength - 12 ).fill( projectrtp.codecx.linear162pcmu( sn ) & 0xff )
    }

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


describe( "record", function() {

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
    expect( wavinfo.chunksize ).to.be.within( 28000, 29000 )
    expect( wavinfo.fmtchunksize ).to.equal( 16 )
    expect( wavinfo.subchunksize ).to.be.within( 28000, 29000 )

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

  it( `record to file with pause`, async function() {
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
        }
      } )

      expect( channel.record( {
        "file": "/tmp/ourpausedrecording.wav"
      } ) ).to.be.true

      /* something to record */
      expect( channel.echo() ).to.be.true

      for( let i = 0;  i < 50; i ++ ) {
        sendpk( i, i, channel.port, server )
      }

      /* @ 400mS pause the recording - 200mS will still be in the buffer*/
      setTimeout( () => {
        expect( channel.record( {
          "file": "/tmp/ourpausedrecording.wav",
          "pause": true
        } ) ).to.be.true
      }, 400 )
    } )

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 1100 ) } )
    channel.close()

    /* Now test the file */
    let wavinfo = projectrtp.soundfile.info( "/tmp/ourpausedrecording.wav" )
    expect( wavinfo.audioformat ).to.equal( 1 )
    expect( wavinfo.channelcount ).to.equal( 2 )
    expect( wavinfo.samplerate ).to.equal( 8000 )
    expect( wavinfo.byterate ).to.equal( 32000 )
    expect( wavinfo.bitdepth ).to.equal( 16 )
    expect( wavinfo.chunksize ).to.be.within( 5500, 7000 ) /* 200mS of audio */
    expect( wavinfo.fmtchunksize ).to.equal( 16 )
    expect( wavinfo.subchunksize ).to.be.within( 5500, 7000 )

    let stats = fs.statSync( "/tmp/ourpausedrecording.wav" )
    expect( stats.size ).to.be.within( 5500, 7000 )

  } )

  it( `record with power detection`, function( done ) {

    this.timeout( 9000 )
    this.slow( 8500 )

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    var receviedpkcount = 0
    var channel

    /* generate our data */
    const startsilenceseconds = 2
    const tonedurationseconds = 2
    const endsilnceseconds = 3
    const totalseconds = startsilenceseconds + tonedurationseconds + endsilnceseconds
    const amplitude = 20000
    const frequescyhz = 50
    const samplingrate = 8000

    let sendbuffer = Buffer.concat( [
      Buffer.alloc( samplingrate*startsilenceseconds, projectrtp.codecx.linear162pcmu( 0 ) ),
      genpcmutone( tonedurationseconds, frequescyhz, samplingrate, amplitude ),
      Buffer.alloc( samplingrate*endsilnceseconds, projectrtp.codecx.linear162pcmu( 0 ) )
    ] )

    if( showplots ) {
      let data1 = [ {
        y: pcmubuffer2array( sendbuffer ),
        type: "scatter"
      } ]

      plot.stack( data1 )
      plot.plot()
    }

    server.on( "message", function( msg, rinfo ) {
      receviedpkcount++
    } )

    server.bind()
    let delayedjobs = []
    server.on( "listening", function() {

      channel = projectrtp.rtpchannel.create( { "target": { "address": "localhost", "port": server.address().port, "codec": 0 } }, function( d ) {

        if( "record" === d.action && "finished.belowpower" == d.event ) {
          channel.close()
        } else if( "close" === d.action ) {

          delayedjobs.every( ( id ) => {
            clearTimeout( id )
            return true
          } )
          server.close()

          let stats = fs.statSync( "/tmp/ourpowerrecording.wav" )
          expect( stats.size ).to.be.within( 70000, 80000 )

          done()
        }
      } )

      expect( channel.record( {
        "file": "/tmp/ourpowerrecording.wav",
        "startabovepower": 250,
        "finishbelowpower": 200,
        "minduration": 2000,
        "maxduration": 15000,
        "poweraveragepackets": 20
      } ) ).to.be.true

      /* something to record */
      expect( channel.echo() ).to.be.true

      for( let i = 0;  i < 50*totalseconds; i ++ ) {
        delayedjobs.push(
          sendpk( i, i, channel.port, server, sendbuffer )
        )
      }
    } )
  } )


  it( `record with timeout after power detection`, function( done ) {

    this.timeout( 9000 )
    this.slow( 8500 )

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    var receviedpkcount = 0
    var channel

    /* generate our data */
    const startsilenceseconds = 2
    const tonedurationseconds = 2
    const endsilnceseconds = 3
    const totalseconds = startsilenceseconds + tonedurationseconds + endsilnceseconds
    const amplitude = 20000
    const frequescyhz = 50
    const samplingrate = 8000

    let sendbuffer = Buffer.concat( [
      Buffer.alloc( samplingrate*startsilenceseconds, projectrtp.codecx.linear162pcmu( 0 ) ),
      genpcmutone( tonedurationseconds, frequescyhz, samplingrate, amplitude ),
      Buffer.alloc( samplingrate*endsilnceseconds, projectrtp.codecx.linear162pcmu( 0 ) )
    ] )

    server.on( "message", function( msg, rinfo ) {
      receviedpkcount++
    } )

    server.bind()
    let delayedjobs = []
    server.on( "listening", function() {

      channel = projectrtp.rtpchannel.create( { "target": { "address": "localhost", "port": server.address().port, "codec": 0 } }, function( d ) {

        if( "record" === d.action && "finished.timeout" == d.event ) {
          channel.close()
        } else if( "close" === d.action ) {

          delayedjobs.every( ( id ) => {
            clearTimeout( id )
            return true
          } )
          server.close()

          let stats = fs.statSync( "/tmp/ourtimeoutpowerrecording.wav" )
          expect( stats.size ).to.be.within( 16000, 18000 )

          done()
        }
      } )

      expect( channel.record( {
        "file": "/tmp/ourtimeoutpowerrecording.wav",
        "startabovepower": 250,
        "finishbelowpower": 200,
        "minduration": 200,
        "maxduration": 500,
        "poweraveragepackets": 50
      } ) ).to.be.true

      /* something to record */
      expect( channel.echo() ).to.be.true

      for( let i = 0;  i < 50*totalseconds; i ++ ) {
        delayedjobs.push(
          sendpk( i, i, channel.port, server, sendbuffer )
        )
      }
    } )
  } )


  it( `dual recording one with power detect one ongoing`, function( done ) {

    this.timeout( 7000 )
    this.slow( 6000 )

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    var receviedpkcount = 0
    var channel

    /* generate our data */
    const startsilenceseconds = 1
    const tonedurationseconds = 1
    const endsilnceseconds = 2
    const totalseconds = startsilenceseconds + tonedurationseconds + endsilnceseconds
    const amplitude = 20000
    const frequescyhz = 50
    const samplingrate = 8000
    const lowamplitude = 500

    let sendbuffer = Buffer.concat( [
      Buffer.alloc( samplingrate*startsilenceseconds, projectrtp.codecx.linear162pcmu( 0 ) ),
      genpcmutone( tonedurationseconds, frequescyhz, samplingrate, amplitude ),
      genpcmutone( endsilnceseconds, frequescyhz, samplingrate, lowamplitude )
    ] )

    server.on( "message", function( msg, rinfo ) {
      receviedpkcount++
    } )

    server.bind()
    let delayedjobs = []

    /*
      Messages we receive in this order:
    */
    const expectedmessages = [
      { action: 'record', file: '/tmp/dualrecording.wav', event: 'recording' },
      { action: 'record', file: '/tmp/dualrecordingpower.wav', event: 'recording' },
      { action: 'record', file: '/tmp/dualrecordingpower.wav', event: 'finished.belowpower' },
      { action: 'record', file: '/tmp/dualrecording.wav', event: 'finished.channelclosed' },
      { action: 'close' }
    ]
    let expectedmessagecount = 0

    server.on( "listening", function() {

      channel = projectrtp.rtpchannel.create( { "target": { "address": "localhost", "port": server.address().port, "codec": 0 } }, function( d ) {

        expect( d ).to.deep.include( expectedmessages[ expectedmessagecount ] )
        expectedmessagecount++

        if( "record" === d.action && "finished.timeout" == d.event ) {

          expect( d.file ).to.equal( "/tmp/dualrecording.wav" )
          let stats = fs.statSync( "/tmp/dualrecording.wav" )
          expect( stats.size ).to.be.within( 128000, 129000 )

        } else if( "record" === d.action && "finished.belowpower" == d.event ) {
          expect( d.file ).to.equal( "/tmp/dualrecordingpower.wav" )

          let stats = fs.statSync( "/tmp/dualrecordingpower.wav" )
          expect( stats.size ).to.be.within( 37000, 38000 )

        } else if( "close" === d.action ) {
          delayedjobs.every( ( id ) => {
            clearTimeout( id )
            return true
          } )
          server.close()

          expect( expectedmessagecount ).to.equal( expectedmessages.length )
          done()
        }
      } )

      expect( channel.record( {
        "file": "/tmp/dualrecording.wav"
      } ) ).to.be.true

      expect( channel.record( {
        "file": "/tmp/dualrecordingpower.wav",
        "startabovepower": 800,
        "finishbelowpower": 600,
        "minduration": 200,
        "maxduration": 1500,
        "poweraveragepackets": 10 /* faster response */
      } ) ).to.be.true

      /* something to record */
      expect( channel.echo() ).to.be.true

      for( let i = 0;  i < 50*totalseconds; i ++ ) {
        delayedjobs.push(
          sendpk( i, i, channel.port, server, sendbuffer )
        )
      }
    } )

    setTimeout( () => channel.close(), 4000 )
  } )

  after( async () => {
    await new Promise( ( resolve, reject ) => { fs.unlink( "/tmp/ourrecording.wav", ( err ) => { resolve() } ) } )
    await new Promise( ( resolve, reject ) => { fs.unlink( "/tmp/ourpausedrecording.wav", ( err ) => { resolve() } ) } )
    await new Promise( ( resolve, reject ) => { fs.unlink( "/tmp/ourpowerrecording.wav", ( err ) => { resolve() } ) } )
    await new Promise( ( resolve, reject ) => { fs.unlink( "/tmp/ourtimeoutpowerrecording.wav", ( err ) => { resolve() } ) } )
    await new Promise( ( resolve, reject ) => { fs.unlink( "/tmp/dualrecordingpower.wav", ( err ) => { resolve() } ) } )
    await new Promise( ( resolve, reject ) => { fs.unlink( "/tmp/dualrecording.wav", ( err ) => { resolve() } ) } )
  } )
} )
