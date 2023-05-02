

const expect = require( "chai" ).expect
const fs = require( "fs" )
const fspromises = fs.promises
const dgram = require( "dgram" )
const prtp = require( "../../index.js" )


function genpcmutone( durationseconds = 0.25, tonehz = 100, samplerate = 16000, amp = 15000 ) {

  const tonebuffer = Buffer.alloc( samplerate*durationseconds, prtp.projectrtp.codecx.linear162pcmu( 0 ) )

  for( let i = 0; i < tonebuffer.length; i++ ) {
    const val = Math.sin( ( i / samplerate ) * Math.PI * tonehz ) * amp
    tonebuffer[ i ] = prtp.projectrtp.codecx.linear162pcmu( val )
  }

  return tonebuffer
}

function sendpk( sn, sendtime, dstport, server, data = undefined ) {

  const ssrc = 25
  const pklength = 172

  return setTimeout( () => {

    let payload
    if( undefined != data ) {
      const start = sn * 160
      const end = start + 160
      payload = data.subarray( start, end )
    } else {
      payload = Buffer.alloc( pklength - 12 ).fill( prtp.projectrtp.codecx.linear162pcmu( sn ) & 0xff )
    }

    const subheader = Buffer.alloc( 10 )

    const ts = sn * 160

    subheader.writeUInt16BE( ( sn + 100 ) % ( 2**16 ) /* just some offset */ )
    subheader.writeUInt32BE( ts, 2 )
    subheader.writeUInt32BE( ssrc, 6 )

    const rtppacket = Buffer.concat( [
      Buffer.from( [ 0x80, 0x00 ] ),
      subheader,
      payload ] )

    server.send( rtppacket, dstport, "localhost" )
  }, sendtime * 20 )
}


describe( "record", function() {

  it( "record to file", async function() {
    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )

    /** @type { prtp.channel } */
    let channel

    server.on( "message", function() {} )

    this.timeout( 1500 )
    this.slow( 1200 )

    server.bind()
    server.on( "listening", async function() {

      channel = await prtp.projectrtp.openchannel( { "remote": { "address": "localhost", "port": server.address().port, "codec": 0 } }, function( d ) {
        if( "close" === d.action ) {
          server.close()
        }
      } )

      expect( channel.record( {
        "file": "/tmp/ourrecording.wav"
      } ) ).to.be.true

      /* something to record */
      expect( channel.echo() ).to.be.true

      for( let i = 0;  50 > i; i ++ ) {
        sendpk( i, i, channel.local.port, server )
      }
    } )

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 1300 ) } )
    channel.close()

    /* Now test the file */
    const wavinfo = prtp.projectrtp.soundfile.info( "/tmp/ourrecording.wav" )
    expect( wavinfo.audioformat ).to.equal( 1 )
    expect( wavinfo.channelcount ).to.equal( 2 )
    expect( wavinfo.samplerate ).to.equal( 8000 )
    expect( wavinfo.byterate ).to.equal( 32000 )
    expect( wavinfo.bitdepth ).to.equal( 16 )
    expect( wavinfo.chunksize ).to.be.within( 28000, 33000 )
    expect( wavinfo.fmtchunksize ).to.equal( 16 )
    expect( wavinfo.subchunksize ).to.be.within( 28000, 33000 )

    const ourfile = await fspromises.open( "/tmp/ourrecording.wav", "r" )
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
    for( let sn = 0; 42 > sn; sn++ ){
      expect( buffer.readInt16BE( 160 * 2 * 2 * sn ) )
        .to.equal( prtp.projectrtp.codecx.pcmu2linear16( prtp.projectrtp.codecx.linear162pcmu( sn ) ) )
    }
  } )

  it( "record to file then request finish", async function() {
    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )

    /** @type { prtp.channel } */
    let channel

    server.on( "message", function() {} )

    this.timeout( 1500 )
    this.slow( 1200 )

    server.bind()

    let done
    const finished = new Promise( ( r ) => { done = r } )

    let expectedmessagecount = 0
    const expectedmessages = [
      { action: "record", file: "/tmp/ourstoppedrecording.wav", event: "recording" },
      { action: "record", file: "/tmp/ourstoppedrecording.wav", event: "finished.requested" },
      { action: "close" }
    ]

    server.on( "listening", async function() {

      channel = await prtp.projectrtp.openchannel( { "remote": { "address": "localhost", "port": server.address().port, "codec": 0 } }, function( d ) {
        expect( d ).to.deep.include( expectedmessages[ expectedmessagecount ] )
        expectedmessagecount++
        if( "close" === d.action ) {
          done()
          server.close()
        }
      } )

      expect( channel.record( {
        "file": "/tmp/ourstoppedrecording.wav"
      } ) ).to.be.true

      /* something to record */
      expect( channel.echo() ).to.be.true

      for( let i = 0;  50 > i; i ++ ) {
        sendpk( i, i, channel.local.port, server )
      }
    } )

    setTimeout( () => channel.record( {
      "file": "/tmp/ourstoppedrecording.wav",
      "finish": true
    } ) , 600 )

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 1300 ) } )
    channel.close()
    await finished

    /* Now test the file */
    const wavinfo = prtp.projectrtp.soundfile.info( "/tmp/ourstoppedrecording.wav" )

    expect( wavinfo.audioformat ).to.equal( 1 )
    expect( wavinfo.channelcount ).to.equal( 2 )
    expect( wavinfo.samplerate ).to.equal( 8000 )
    expect( wavinfo.byterate ).to.equal( 32000 )
    expect( wavinfo.bitdepth ).to.equal( 16 )
    expect( wavinfo.chunksize ).to.be.within( 8000, 13000 )
    expect( wavinfo.fmtchunksize ).to.equal( 16 )
    expect( wavinfo.subchunksize ).to.be.within( 8000, 13000 )

  } )

  it( "record to file with pause", async function() {
    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )

    /** @type { prtp.channel } */
    let channel

    server.on( "message", function() {} )

    this.timeout( 1500 )
    this.slow( 1200 )

    let done
    const finished = new Promise( ( r ) => { done = r } )

    server.bind()
    server.on( "listening", async function() {

      channel = await prtp.projectrtp.openchannel( { "remote": { "address": "localhost", "port": server.address().port, "codec": 0 } }, function( d ) {
        if( "close" === d.action ) {
          server.close()
          done()
        }
      } )

      expect( channel.record( {
        "file": "/tmp/ourpausedrecording.wav"
      } ) ).to.be.true

      /* something to record */
      expect( channel.echo() ).to.be.true

      for( let i = 0;  50 > i; i ++ ) {
        sendpk( i, i, channel.local.port, server )
      }

      /* @ 400mS pause the recording - 200mS will still be in the buffer*/
      setTimeout( () => {
        expect( channel.record( {
          "file": "/tmp/ourpausedrecording.wav",
          "pause": true
        } ) ).to.be.true
      }, 400 )
    } )

    await new Promise( ( r ) => { setTimeout( () => r(), 1300 ) } )
    channel.close()

    await finished

    /* Now test the file */
    const wavinfo = prtp.projectrtp.soundfile.info( "/tmp/ourpausedrecording.wav" )
    expect( wavinfo.audioformat ).to.equal( 1 )
    expect( wavinfo.channelcount ).to.equal( 2 )
    expect( wavinfo.samplerate ).to.equal( 8000 )
    expect( wavinfo.byterate ).to.equal( 32000 )
    expect( wavinfo.bitdepth ).to.equal( 16 )
    expect( wavinfo.chunksize ).to.be.within( 3000, 7000 ) /* 200mS of audio */
    expect( wavinfo.fmtchunksize ).to.equal( 16 )
    expect( wavinfo.subchunksize ).to.be.within( 2500, 7000 )

    const stats = fs.statSync( "/tmp/ourpausedrecording.wav" )
    expect( stats.size ).to.be.within( 2500, 7000 )

  } )

  it( "record with power detection", function( done ) {

    this.timeout( 9000 )
    this.slow( 8500 )

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    let channel

    /* generate our data */
    const startsilenceseconds = 2
    const tonedurationseconds = 2
    const endsilnceseconds = 3
    const totalseconds = startsilenceseconds + tonedurationseconds + endsilnceseconds
    const amplitude = 20000
    const frequescyhz = 50
    const samplingrate = 8000

    const sendbuffer = Buffer.concat( [
      Buffer.alloc( samplingrate*startsilenceseconds, prtp.projectrtp.codecx.linear162pcmu( 0 ) ),
      genpcmutone( tonedurationseconds, frequescyhz, samplingrate, amplitude ),
      Buffer.alloc( samplingrate*endsilnceseconds, prtp.projectrtp.codecx.linear162pcmu( 0 ) )
    ] )

    server.on( "message", function() {} )

    let expectedmessagecount = 0
    const expectedmessages = [
      { action: "record", file: "/tmp/ourpowerrecording.wav", event: "recording.abovepower" },
      { action: "record", file: "/tmp/ourpowerrecording.wav", event: "finished.belowpower" },
      { action: "close" }
    ]

    server.bind()
    const delayedjobs = []
    server.on( "listening", async function() {

      channel = await prtp.projectrtp.openchannel( { "remote": { "address": "localhost", "port": server.address().port, "codec": 0 } }, function( d ) {

        expect( d ).to.deep.include( expectedmessages[ expectedmessagecount ] )
        expectedmessagecount++

        if( 2 == expectedmessagecount ) {
          channel.close()
        } else if( 3 == expectedmessagecount ) {

          delayedjobs.every( ( id ) => {
            clearTimeout( id )
            return true
          } )
          server.close()
          const stats = fs.statSync( "/tmp/ourpowerrecording.wav" )
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
          sendpk( i, i, channel.local.port, server, sendbuffer )
        )
      }
    } )
  } )


  it( "record with timeout after power detection", function( done ) {

    this.timeout( 9000 )
    this.slow( 8500 )

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    let channel

    /* generate our data */
    const startsilenceseconds = 2
    const tonedurationseconds = 2
    const endsilnceseconds = 3
    const totalseconds = startsilenceseconds + tonedurationseconds + endsilnceseconds
    const amplitude = 20000
    const frequescyhz = 50
    const samplingrate = 8000

    const sendbuffer = Buffer.concat( [
      Buffer.alloc( samplingrate*startsilenceseconds, prtp.projectrtp.codecx.linear162pcmu( 0 ) ),
      genpcmutone( tonedurationseconds, frequescyhz, samplingrate, amplitude ),
      Buffer.alloc( samplingrate*endsilnceseconds, prtp.projectrtp.codecx.linear162pcmu( 0 ) )
    ] )

    server.on( "message", function() {} )

    server.bind()
    const delayedjobs = []
    server.on( "listening", async function() {

      channel = await prtp.projectrtp.openchannel( { "remote": { "address": "localhost", "port": server.address().port, "codec": 0 } }, function( d ) {

        if( "record" === d.action && "finished.timeout" == d.event ) {
          channel.close()
        } else if( "close" === d.action ) {

          delayedjobs.every( ( id ) => {
            clearTimeout( id )
            return true
          } )
          server.close()

          const stats = fs.statSync( "/tmp/ourtimeoutpowerrecording.wav" )
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
          sendpk( i, i, channel.local.port, server, sendbuffer )
        )
      }
    } )
  } )


  it( "dual recording one with power detect one ongoing", async function () {

    this.timeout( 8000 )
    this.slow( 7000 )

    let done
    const finished = new Promise( ( r ) => done = r )

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    let channel

    /* generate our data */
    const startsilenceseconds = 1
    const tonedurationseconds = 2
    const endsilnceseconds = 2
    const totalseconds = startsilenceseconds + tonedurationseconds + endsilnceseconds
    const amplitude = 20000
    const frequescyhz = 50
    const samplingrate = 8000
    const lowamplitude = 200

    const sendbuffer = Buffer.concat( [
      Buffer.alloc( samplingrate*startsilenceseconds, prtp.projectrtp.codecx.linear162pcmu( 0 ) ),
      genpcmutone( tonedurationseconds, frequescyhz, samplingrate, amplitude ),
      genpcmutone( endsilnceseconds, frequescyhz, samplingrate, lowamplitude )
    ] )

    server.on( "message", function() {} )

    server.bind()
    const receivedmessages = []

    server.on( "listening", async function() {

      channel = await prtp.projectrtp.openchannel( { "remote": { "address": "localhost", "port": server.address().port, "codec": 0 } }, function( d ) {
        receivedmessages.push( d )  
        if( "close" === d.action ) {
          done()
        }
      } )

      expect( channel.record( {
        "file": "/tmp/dualrecording.wav"
      } ) ).to.be.true

      expect( channel.record( {
        "file": "/tmp/dualrecordingpower.wav",
        "startabovepower": 200,
        "finishbelowpower": 180,
        "minduration": 200,
        "maxduration": 3500,
        "poweraveragepackets": 10 /* faster response */
      } ) ).to.be.true

      /* something to record */
      expect( channel.echo() ).to.be.true

      for( let i = 0;  i < 50*totalseconds; i ++ ) {
        sendpk( i, i, channel.local.port, server, sendbuffer )
      }
    } )

    setTimeout( () => channel.close(), totalseconds * 1000 )
    await finished

    server.close()

    let stats = fs.statSync( "/tmp/dualrecordingpower.wav" )
    expect( stats.size ).to.be.within( 40000, 52000 )

    stats = fs.statSync( "/tmp/dualrecording.wav" )
    expect( stats.size ).to.be.within( 110000, 190000 )

    /*
      Messages we receive in this order:
    */
    const expectedmessages = [
      { action: "record", file: "/tmp/dualrecording.wav", event: "recording" },
      { action: "record", file: "/tmp/dualrecordingpower.wav", event: "recording.abovepower" },
      { action: "record", file: "/tmp/dualrecordingpower.wav", event: "finished.belowpower" },
      { action: "record", file: "/tmp/dualrecording.wav", event: "finished.channelclosed" },
      { action: "close" }
    ]

    for( let i = 0; i < expectedmessages.length; i ++ ) {
      expect( expectedmessages[ i ].action ).to.equal( receivedmessages[ i ].action )
      if( expectedmessages[ i ].file ) expect( expectedmessages[ i ].file ).to.equal( receivedmessages[ i ].file )
      if( expectedmessages[ i ].event ) expect( expectedmessages[ i ].event ).to.equal( receivedmessages[ i ].event )
    }
  } )

  after( async () => {
    await new Promise( ( resolve ) => { fs.unlink( "/tmp/ourrecording.wav", () => { resolve() } ) } )
    await new Promise( ( resolve ) => { fs.unlink( "/tmp/ourstoppedrecording.wav", () => { resolve() } ) } )
    await new Promise( ( resolve ) => { fs.unlink( "/tmp/ourpausedrecording.wav", () => { resolve() } ) } )
    await new Promise( ( resolve ) => { fs.unlink( "/tmp/ourpowerrecording.wav", () => { resolve() } ) } )
    await new Promise( ( resolve ) => { fs.unlink( "/tmp/ourtimeoutpowerrecording.wav", () => { resolve() } ) } )
    await new Promise( ( resolve ) => { fs.unlink( "/tmp/dualrecordingpower.wav", () => { resolve() } ) } )
    await new Promise( ( resolve ) => { fs.unlink( "/tmp/dualrecording.wav", () => { resolve() } ) } )
  } )
} )
