

const expect = require( "chai" ).expect
const fs = require( "fs" )
const dgram = require( "dgram" )
const prtp = require( "../../index.js" )


function genpcmutone( durationseconds = 0.25, tonehz = 100, samplerate = 8000, amp = 15000 ) {
  const tonebuffer = Buffer.alloc( samplerate * durationseconds, prtp.projectrtp.codecx.linear162pcmu( 0 ) )
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
    subheader.writeUInt16BE( ( sn + 100 ) % ( 2**16 ) )
    subheader.writeUInt32BE( ts, 2 )
    subheader.writeUInt32BE( ssrc, 6 )

    const rtppacket = Buffer.concat( [
      Buffer.from( [ 0x80, 0x00 ] ),
      subheader,
      payload
    ] )

    server.send( rtppacket, dstport, "localhost" )
  }, sendtime * 20 )
}

/* create a short wav file for playback */
function createwavfile( filepath, samples = 4000 ) {
  const bytespersample = 2
  const samplerate = 8000
  const numchannels = 1
  const header = Buffer.alloc( 44 )

  header.write( "RIFF", 0 )
  header.writeUInt32LE( 36 + samples * bytespersample, 4 )
  header.write( "WAVE", 8 )
  header.write( "fmt ", 12 )
  header.writeUInt32LE( 16, 16 )
  header.writeUInt16LE( 1, 20 ) /* PCM */
  header.writeUInt16LE( numchannels, 22 )
  header.writeUInt32LE( samplerate, 24 )
  header.writeUInt32LE( samplerate * numchannels * bytespersample, 28 )
  header.writeUInt16LE( numchannels * bytespersample, 32 )
  header.writeUInt16LE( bytespersample * 8, 34 )
  header.write( "data", 36 )
  header.writeUInt32LE( samples * bytespersample, 40 )

  const data = Buffer.alloc( samples * bytespersample )
  for( let i = 0; i < samples; i++ ) {
    data.writeInt16LE( Math.sin( i * Math.PI * 2 * 440 / samplerate ) * 10000, i * 2 )
  }

  fs.writeFileSync( filepath, Buffer.concat( [ header, data ] ) )
}


describe( "playrecord", function() {

  before( function() {
    createwavfile( "/tmp/pr_prompt.wav", 4000 ) /* 500ms at 8kHz */
    createwavfile( "/tmp/pr_long_prompt.wav", 24000 ) /* 3s at 8kHz */
  } )

  it( "basic playrecord — play then record with zero gap", async function() {
    this.timeout( 4000 )
    this.slow( 3000 )

    const server = dgram.createSocket( "udp4" )
    server.on( "message", function() {} )
    server.bind()
    await new Promise( resolve => server.on( "listening", resolve ) )

    const events = []
    let done
    const finished = new Promise( r => done = r )

    const channel = await prtp.projectrtp.openchannel(
      { "remote": { "address": "127.0.0.1", "port": server.address().port, "codec": 0 } },
      function( d ) {
        events.push( { ...d, ts: Date.now() } )
        if( "close" === d.action ) done()
      }
    )

    expect( channel.playrecord( {
      "soup": { "files": [ { "wav": "/tmp/pr_prompt.wav" } ] },
      "record": {
        "file": "/tmp/pr_basic_rec.wav",
        "numchannels": 1,
        "maxduration": 2000
      }
    } ) ).to.be.true

    /* send RTP packets throughout — these are what gets recorded */
    for( let i = 0; i < 150; i++ ) {
      sendpk( i, i, channel.local.port, server )
    }

    await new Promise( resolve => setTimeout( resolve, 3500 ) )
    channel.close()
    await finished
    await new Promise( resolve => server.close( resolve ) )

    /* verify event sequence */
    const playstart = events.find( e => "play" === e.action && "start" === e.event )
    const playend = events.find( e => "play" === e.action && "end" === e.event )
    const recstart = events.find( e => "record" === e.action && "recording" === e.event )

    expect( playstart ).to.exist
    expect( playend ).to.exist
    expect( playend.reason ).to.equal( "completed" )
    expect( recstart ).to.exist

    /* zero gap: record should start in the same tick batch as play end */
    expect( recstart.ts - playend.ts ).to.be.below( 25 )

    /* verify recorded file */
    expect( fs.existsSync( "/tmp/pr_basic_rec.wav" ) ).to.be.true
    const wavinfo = prtp.projectrtp.soundfile.info( "/tmp/pr_basic_rec.wav" )
    expect( wavinfo.audioformat ).to.equal( 1 )
    expect( wavinfo.channelcount ).to.equal( 1 )
    expect( wavinfo.samplerate ).to.equal( 8000 )
  } )

  it( "barge-in — loud audio during play triggers interrupt", async function() {
    this.timeout( 8000 )
    this.slow( 6000 )

    const server = dgram.createSocket( "udp4" )
    server.on( "message", function() {} )
    server.bind()
    await new Promise( resolve => server.on( "listening", resolve ) )

    const events = []
    let done
    const finished = new Promise( r => done = r )

    const channel = await prtp.projectrtp.openchannel(
      { "remote": { "address": "127.0.0.1", "port": server.address().port, "codec": 0 } },
      function( d ) {
        events.push( d )
        if( "close" === d.action ) done()
      }
    )

    expect( channel.playrecord( {
      "soup": { "files": [ { "wav": "/tmp/pr_long_prompt.wav" } ] },
      "record": {
        "file": "/tmp/pr_bargein_rec.wav",
        "numchannels": 1,
        "maxduration": 3000
      },
      "interrupt": true,
      "bargeinpower": 100,
      "bargeinpoweraveragepackets": 5
    } ) ).to.be.true

    /* send silence for 1.5s then loud tone */
    const silencesamples = 8000 * 1.5
    const tonesamples = 8000 * 2
    const sendbuffer = Buffer.concat( [
      Buffer.alloc( silencesamples, prtp.projectrtp.codecx.linear162pcmu( 0 ) ),
      genpcmutone( 2, 50, 8000, 20000 )
    ] )

    const totalpackets = Math.ceil( ( silencesamples + tonesamples ) / 160 )
    const delayedjobs = []
    for( let i = 0; i < totalpackets; i++ ) {
      delayedjobs.push( sendpk( i, i, channel.local.port, server, sendbuffer ) )
    }

    await new Promise( resolve => setTimeout( resolve, 6000 ) )
    channel.close()
    await finished
    delayedjobs.forEach( id => clearTimeout( id ) )
    await new Promise( resolve => server.close( resolve ) )

    /* verify barge-in occurred */
    const playend = events.find( e => "play" === e.action && "end" === e.event )
    expect( playend ).to.exist
    expect( playend.reason ).to.equal( "interrupted" )

    const recstart = events.find( e => "record" === e.action && "recording" === e.event )
    expect( recstart ).to.exist

    expect( fs.existsSync( "/tmp/pr_bargein_rec.wav" ) ).to.be.true
  } )

  it( "no barge-in on silence — play completes normally", async function() {
    this.timeout( 5000 )
    this.slow( 4000 )

    const server = dgram.createSocket( "udp4" )
    server.on( "message", function() {} )
    server.bind()
    await new Promise( resolve => server.on( "listening", resolve ) )

    const events = []
    let done
    const finished = new Promise( r => done = r )

    const channel = await prtp.projectrtp.openchannel(
      { "remote": { "address": "127.0.0.1", "port": server.address().port, "codec": 0 } },
      function( d ) {
        events.push( d )
        if( "close" === d.action ) done()
      }
    )

    expect( channel.playrecord( {
      "soup": { "files": [ { "wav": "/tmp/pr_prompt.wav" } ] },
      "record": {
        "file": "/tmp/pr_silence_rec.wav",
        "numchannels": 1,
        "maxduration": 1500
      },
      "interrupt": true,
      "bargeinpower": 100,
      "bargeinpoweraveragepackets": 5
    } ) ).to.be.true

    /* send only silence */
    const silencebuf = Buffer.alloc( 8000 * 3, prtp.projectrtp.codecx.linear162pcmu( 0 ) )
    for( let i = 0; i < 150; i++ ) {
      sendpk( i, i, channel.local.port, server, silencebuf )
    }

    await new Promise( resolve => setTimeout( resolve, 3500 ) )
    channel.close()
    await finished
    await new Promise( resolve => server.close( resolve ) )

    /* play should have completed normally, not interrupted */
    const playend = events.find( e => "play" === e.action && "end" === e.event )
    expect( playend ).to.exist
    expect( playend.reason ).to.equal( "completed" )

    const recstart = events.find( e => "record" === e.action && "recording" === e.event )
    expect( recstart ).to.exist
  } )

  it( "playrecord with record finish request", async function() {
    this.timeout( 3000 )
    this.slow( 2500 )

    const server = dgram.createSocket( "udp4" )
    server.on( "message", function() {} )
    server.bind()
    await new Promise( resolve => server.on( "listening", resolve ) )

    const events = []
    let done
    const finished = new Promise( r => done = r )

    const channel = await prtp.projectrtp.openchannel(
      { "remote": { "address": "127.0.0.1", "port": server.address().port, "codec": 0 } },
      function( d ) {
        events.push( d )
        if( "close" === d.action ) done()
      }
    )

    expect( channel.playrecord( {
      "soup": { "files": [ { "wav": "/tmp/pr_prompt.wav" } ] },
      "record": {
        "file": "/tmp/pr_finish_rec.wav",
        "numchannels": 1
      }
    } ) ).to.be.true

    for( let i = 0; i < 100; i++ ) {
      sendpk( i, i, channel.local.port, server )
    }

    /* wait for record to activate then request finish */
    setTimeout( () => {
      channel.record( { "file": "/tmp/pr_finish_rec.wav", "finish": true } )
    }, 1500 )

    await new Promise( resolve => setTimeout( resolve, 2500 ) )
    channel.close()
    await finished
    await new Promise( resolve => server.close( resolve ) )

    const recfinish = events.find( e => "record" === e.action && "finished.requested" === e.event )
    expect( recfinish ).to.exist
  } )

  it( "close during playrecord — clean shutdown", async function() {
    this.timeout( 2000 )
    this.slow( 1500 )

    const server = dgram.createSocket( "udp4" )
    server.on( "message", function() {} )
    server.bind()
    await new Promise( resolve => server.on( "listening", resolve ) )

    let done
    const finished = new Promise( r => done = r )

    const channel = await prtp.projectrtp.openchannel(
      { "remote": { "address": "127.0.0.1", "port": server.address().port, "codec": 0 } },
      function( d ) {
        if( "close" === d.action ) done()
      }
    )

    expect( channel.playrecord( {
      "soup": { "files": [ { "wav": "/tmp/pr_long_prompt.wav" } ] },
      "record": {
        "file": "/tmp/pr_close_rec.wav",
        "numchannels": 1
      }
    } ) ).to.be.true

    /* close immediately during playback */
    setTimeout( () => channel.close(), 200 )

    await finished
    await new Promise( resolve => server.close( resolve ) )
    /* no crash = pass */
  } )

  after( async function() {
    const files = [
      "/tmp/pr_prompt.wav",
      "/tmp/pr_long_prompt.wav",
      "/tmp/pr_basic_rec.wav",
      "/tmp/pr_bargein_rec.wav",
      "/tmp/pr_silence_rec.wav",
      "/tmp/pr_finish_rec.wav",
      "/tmp/pr_close_rec.wav"
    ]
    for( const f of files ) {
      await fs.promises.unlink( f ).catch( () => {} )
    }
  } )
} )
