/*
  Recordings and read streams on ticks with no packet from the caller (RTP-28).

  A channel decodes the caller's packets and encodes its own outbound audio
  (a prompt, or a conference's mix) through the same codec object, which keeps
  its last decode until the next one. On a tick with no packet, that is either
  the caller's previous frame or audio the channel sent, so the caller's side
  of a recording or read stream must be silence then, not whatever the codec
  last held.
*/

const expect = require( "chai" ).expect
const fs = require( "fs" )
const dgram = require( "dgram" )
const prtp = require( "../../index.js" )

const tone1k = "/tmp/np_tone1k.wav"
const tone440 = "/tmp/np_tone440.wav"

/**
 * Write a mono 8 kHz WAV holding a sine tone.
 * @param { string } filepath
 * @param { number } samples
 * @param { number } tonehz
 */
function createwavfile( filepath, samples, tonehz ) {
  const samplerate = 8000
  const header = Buffer.alloc( 44 )
  header.write( "RIFF", 0 )
  header.writeUInt32LE( 36 + ( samples * 2 ), 4 )
  header.write( "WAVE", 8 )
  header.write( "fmt ", 12 )
  header.writeUInt32LE( 16, 16 )
  header.writeUInt16LE( 1, 20 )
  header.writeUInt16LE( 1, 22 )
  header.writeUInt32LE( samplerate, 24 )
  header.writeUInt32LE( samplerate * 2, 28 )
  header.writeUInt16LE( 2, 32 )
  header.writeUInt16LE( 16, 34 )
  header.write( "data", 36 )
  header.writeUInt32LE( samples * 2, 40 )

  const data = Buffer.alloc( samples * 2 )
  for( let i = 0; i < samples; i++ ) {
    data.writeInt16LE( Math.round( Math.sin( i * Math.PI * 2 * tonehz / samplerate ) * 10000 ), i * 2 )
  }
  fs.writeFileSync( filepath, Buffer.concat( [ header, data ] ) )
}

/**
 * @param { number } ms
 */
function wait( ms ) {
  return new Promise( resolve => setTimeout( resolve, ms ) )
}

/**
 * Little-endian 16-bit samples from a buffer.
 * @param { Buffer } buf
 * @returns { Array< number > }
 */
function samplesof( buf ) {
  const out = []
  for( let i = 0; i + 1 < buf.length; i += 2 ) out.push( buf.readInt16LE( i ) )
  return out
}

/**
 * @param { Array< number > } samples
 */
function peak( samples ) {
  let p = 0
  for( const s of samples ) p = Math.max( p, Math.abs( s ) )
  return p
}

/**
 * Tone frequency from zero crossings.
 * @param { Array< number > } samples
 * @param { number } samplerate
 */
function tonehz( samples, samplerate ) {
  let crossings = 0
  for( let i = 1; i < samples.length; i++ ) {
    if( ( 0 > samples[ i - 1 ] ) !== ( 0 > samples[ i ] ) ) crossings++
  }
  return crossings / 2 / ( samples.length / samplerate )
}

/**
 * Opens channels and tracks their close events, plus a UDP socket that
 * swallows whatever a channel sends to its remote.
 */
async function harness() {
  const sink = dgram.createSocket( "udp4" )
  sink.on( "message", function() {} )
  sink.bind()
  await new Promise( resolve => sink.on( "listening", resolve ) )

  const channels = []
  const closed = []
  return {
    "sinkport": sink.address().port,
    /**
     * @param { number } port
     * @param { number } codec
     */
    "open": async ( port, codec ) => {
      let done
      closed.push( new Promise( r => done = r ) )
      const chan = await prtp.projectrtp.openchannel(
        { "remote": { "address": "127.0.0.1", "port": port, "codec": codec } },
        function( d ) { if( "close" === d.action ) done() }
      )
      channels.push( chan )
      return chan
    },
    "closeall": async () => {
      for( const chan of channels ) chan.close()
      await Promise.all( closed )
      await /** @type { Promise< void > } */ ( new Promise( resolve => sink.close( resolve ) ) )
    }
  }
}

/**
 * Collect a read stream's frames, with the time each arrived.
 * @param { import( "stream" ).Readable } reader
 */
function collect( reader ) {
  /** @type { Array< { at: number, samples: Array< number > } > } */
  const frames = []
  reader.on( "data", ( buf ) => frames.push( { "at": Date.now(), "samples": samplesof( buf ) } ) )
  return {
    frames,
    /**
     * Samples of the frames that arrived in [from, to) ms.
     * @param { number } from
     * @param { number } to
     */
    "between": ( from, to ) => frames.filter( f => f.at >= from && f.at < to ).flatMap( f => f.samples ),
    /**
     * Samples of the frames from index `i` on.
     * @param { number } i
     */
    "fromframe": ( i ) => frames.slice( i ).flatMap( f => f.samples )
  }
}

/**
 * The frame from which an 8 kHz in stream stayed silent once the far end
 * stopped sending a tone: packets already in the jitter buffer play out
 * first. The 8 kHz stream was always silent on a tick with no packet, so
 * from there on no tick has one. Both streams get one frame per tick, so
 * frames are matched by index, not by arrival time (each stream reaches JS
 * through its own queue). The margin covers the two readers being
 * registered a tick or two apart.
 * @param { ReturnType< typeof collect > } r8
 */
function silentfrom( r8 ) {
  let last = -1
  r8.frames.forEach( ( f, i ) => { if( 0 < peak( f.samples ) ) last = i } )
  return last + 3
}

describe( "no packet from the caller this tick", function() {

  this.timeout( 6000 )
  this.slow( 3000 )

  before( function() {
    createwavfile( tone1k, 8000, 1000 )
    createwavfile( tone440, 8000, 440 )
  } )

  after( function() {
    for( const f of [ tone1k, tone440 ] ) {
      if( fs.existsSync( f ) ) fs.unlinkSync( f )
    }
  } )

  it( "local G.722: an in recording made while a prompt plays is silent when the caller sends nothing", async function() {
    const h = await harness()
    const file = "/tmp/np_g722_in_rec.wav"
    const caller = await h.open( h.sinkport, 9 )

    expect( caller.play( { "loop": true, "files": [ { "wav": tone1k } ] } ) ).to.be.true
    expect( caller.record( { "file": file, "direction": "in", "numchannels": 1 } ) ).to.be.true
    await wait( 600 )
    await h.closeall()

    expect( prtp.projectrtp.soundfile.info( file ).samplerate ).to.equal( 16000 )
    const samples = samplesof( fs.readFileSync( file ).subarray( 44 ) )
    fs.unlinkSync( file )
    /* the prompt's frames were written, as silence on the caller's side */
    expect( samples.length ).to.be.above( 16000 * 0.3 )
    expect( peak( samples ) ).to.equal( 0 )
  } )

  it( "local: a 16 kHz in read stream doesn't carry the channel's own prompt", async function() {
    const h = await harness()
    const caller = await h.open( h.sinkport, 0 )
    expect( caller.play( { "loop": true, "files": [ { "wav": tone1k } ] } ) ).to.be.true
    const r16 = collect( caller.createReadStream( { "direction": "in", "samplerate": 16000 } ) )
    await wait( 500 )
    await h.closeall()

    /* silence frames keep coming, as they do from an 8 kHz reader */
    expect( r16.frames.length ).to.be.above( 10 )
    expect( peak( r16.between( 0, Infinity ) ) ).to.equal( 0 )
  } )

  it( "local: 16 kHz and 8 kHz in read streams go silent when the caller stops sending", async function() {
    const h = await harness()
    const caller = await h.open( h.sinkport, 0 )
    const farend = await h.open( caller.local.port, 0 )
    expect( farend.play( { "loop": true, "files": [ { "wav": tone1k } ] } ) ).to.be.true
    const r16 = collect( caller.createReadStream( { "direction": "in", "samplerate": 16000 } ) )
    const r8 = collect( caller.createReadStream( { "direction": "in", "samplerate": 8000 } ) )

    await wait( 500 )
    farend.close()
    const stopped = Date.now()
    await wait( 800 )
    await h.closeall()

    /* the tone came through while the far end sent it */
    expect( peak( r16.between( 0, stopped ) ) ).to.be.above( 1000 )
    expect( peak( r8.between( 0, stopped ) ) ).to.be.above( 1000 )
    /* and once the packets already queued have played out, both are silent */
    const settled = silentfrom( r8 )
    expect( r8.frames.length - settled ).to.be.above( 10 )
    expect( r16.frames.length - settled ).to.be.above( 10 )
    expect( peak( r16.fromframe( settled ) ) ).to.equal( 0 )
  } )

  it( "mixed: a 16 kHz in read stream goes silent when its leg stops sending and the other leg doesn't", async function() {
    const h = await harness()
    const a = await h.open( h.sinkport, 0 )
    const b = await h.open( h.sinkport, 0 )
    const fara = await h.open( a.local.port, 0 )
    const farb = await h.open( b.local.port, 0 )
    expect( a.mix( b ) ).to.be.true
    expect( fara.play( { "loop": true, "files": [ { "wav": tone1k } ] } ) ).to.be.true
    expect( farb.play( { "loop": true, "files": [ { "wav": tone440 } ] } ) ).to.be.true
    const r16 = collect( a.createReadStream( { "direction": "in", "samplerate": 16000 } ) )
    const r8 = collect( a.createReadStream( { "direction": "in", "samplerate": 8000 } ) )

    await wait( 500 )
    fara.close()
    const stopped = Date.now()
    await wait( 800 )
    await h.closeall()

    expect( peak( r16.between( 0, stopped ) ) ).to.be.above( 1000 )
    const settled = silentfrom( r8 )
    expect( r8.frames.length - settled ).to.be.above( 10 )
    expect( r16.frames.length - settled ).to.be.above( 10 )
    expect( peak( r16.fromframe( settled ) ) ).to.equal( 0 )
  } )

  it( "conference: a leg's in recording and 16 kHz read stream hold that leg's audio, not the others'", async function() {
    const h = await harness()
    const file = "/tmp/np_conf_in_rec.wav"
    const a = await h.open( h.sinkport, 0 )
    const b = await h.open( h.sinkport, 0 )
    const c = await h.open( h.sinkport, 0 )
    const fara = await h.open( a.local.port, 0 )
    const farb = await h.open( b.local.port, 0 )
    expect( a.mix( b ) ).to.be.true
    expect( a.mix( c ) ).to.be.true
    expect( fara.play( { "loop": true, "files": [ { "wav": tone1k } ] } ) ).to.be.true
    expect( farb.play( { "loop": true, "files": [ { "wav": tone440 } ] } ) ).to.be.true
    await wait( 200 )

    expect( a.record( { "file": file, "direction": "in", "numchannels": 1 } ) ).to.be.true
    const r16 = collect( a.createReadStream( { "direction": "in", "samplerate": 16000 } ) )
    await wait( 1000 )
    await h.closeall()

    const samples = samplesof( fs.readFileSync( file ).subarray( 44 ) )
    fs.unlinkSync( file )
    expect( samples.length ).to.be.above( 8000 * 0.6 )
    expect( tonehz( samples.slice( 800, 4800 ), 8000 ) ).to.be.within( 900, 1100 )

    const read = r16.between( 0, Infinity )
    expect( read.length ).to.be.above( 16000 * 0.6 )
    expect( tonehz( read.slice( 1600, 9600 ), 16000 ) ).to.be.within( 900, 1100 )
  } )
} )
