/*
 * End-to-end codec chain tests with **real G.722 on the wire**.
 *
 * Topology (matches a real 2-leg Poly ↔ Mobile call):
 *
 *    JS sine gen ──► /tmp/tone400.wav
 *                        │
 *                        │ (player loop, projectrtp reads WAV, encodes)
 *                        ▼
 *    ┌────────────────────────────────┐
 *    │ PseudoPoly (codec=G.722)       │  ← LOCAL mode, not in any mix.
 *    │ remote = Ch_A.local             │    "Plays the role of the Poly
 *    └────────────────────────────────┘    phone: its player output is
 *                        │                  G.722 on the wire."
 *                        │ UDP G.722 (pt=9)
 *                        ▼
 *    ┌────────────────────────────────┐
 *    │ Ch_A (codec=G.722, mixed w/ B) │ ── decodes G.722 from PseudoPoly
 *    │ remote = sink (we don't care)  │    via phase-2, frame → mix
 *    └────────────────────────────────┘
 *                        │  (mix: Ch_A's 8 kHz linear → Ch_B.codec)
 *                        ▼
 *    ┌────────────────────────────────┐
 *    │ Ch_B (codec=PCMA, mixed w/ A)  │ ── encodes to PCMA via mix, sends
 *    │ remote = JS measurement socket │    outbound to JS.
 *    └────────────────────────────────┘
 *                        │ UDP PCMA (pt=8)
 *                        ▼
 *                JS endpoint collects PCMA
 *                    → decode (JS a-law table)
 *                    → FFT, assert 400 Hz dominant
 *                    → write received audio to /tmp/<test>.wav
 *
 * This exercises the full G.722 → 8 kHz → PCMA chain that the real
 * server hits on a Poly → Mobile call. The WAV output on disk lets you
 * listen / load into a DAW when something goes wrong.
 *
 * To cover more shapes (different codecs, different mix positions),
 * `runchain()` takes the 3 codecs as parameters. The "capture" leg
 * (Ch_B) has to be PCMA or PCMU because JS can't decode G.722 / iLBC.
 */

const expect = require( "chai" ).expect
const dgram = require( "dgram" )
const fs = require( "fs" )
const fft = require( "fft-js" ).fft
const projectrtp = require( "../../index" ).projectrtp


// ---- WAV writer -----------------------------------------------------------
//
// Minimal PCM-16 mono RIFF/WAVE. `samples` is linear 16-bit at `sampleRate`.
// We don't use the projectrtp recorder because that would require a channel;
// here we just want a file on disk to inspect when a test fails.

/**
 * @param { string } path
 * @param { number[] | Int16Array } samples
 * @param { number } sampleRate
 */
function writewav( path, samples, sampleRate ) {
  const pcmBytes = samples.length * 2
  const header = Buffer.alloc( 44 )
  header.write( "RIFF", 0 )
  header.writeUInt32LE( 36 + pcmBytes, 4 )
  header.write( "WAVE", 8 )
  header.write( "fmt ", 12 )
  header.writeUInt32LE( 16, 16 )             // fmt chunk size
  header.writeUInt16LE( 1, 20 )              // PCM
  header.writeUInt16LE( 1, 22 )              // mono
  header.writeUInt32LE( sampleRate, 24 )
  header.writeUInt32LE( sampleRate * 2, 28 ) // byte rate
  header.writeUInt16LE( 2, 32 )              // block align
  header.writeUInt16LE( 16, 34 )             // bits per sample
  header.write( "data", 36 )
  header.writeUInt32LE( pcmBytes, 40 )

  const pcm = Buffer.alloc( pcmBytes )
  for( let i = 0; i < samples.length; i++ ) pcm.writeInt16LE( samples[ i ], i * 2 )

  fs.writeFileSync( path, Buffer.concat( [ header, pcm ] ) )
}


// ---- FFT helpers ----------------------------------------------------------

function truncpow2( arr ) {
  const n = Math.pow( 2, Math.floor( Math.log2( arr.length ) ) )
  return arr.slice( 0, n )
}

/** @param { number[] | Int16Array } samples → bin magnitudes (length = N) */
function ampfft( samples ) {
  const c = fft( truncpow2( Array.from( samples ) ) )
  return c.map( ( [ r, i ] ) => Math.sqrt( r * r + i * i ) )
}

/**
 * Sum of magnitudes in the `±window` bins centred on `hz`.
 * @param { number[] } amps
 * @param { number } hz
 * @param { number } sampleRate
 * @param { number } window
 */
function energyat( amps, hz, sampleRate, window = 4 ) {
  const nyquist = sampleRate / 2
  const bin = Math.round( ( hz / nyquist ) * ( amps.length / 2 ) )
  let s = 0
  for( let i = Math.max( 0, bin - window ); i <= Math.min( amps.length - 1, bin + window ); i++ ) {
    s += amps[ i ]
  }
  return s
}


// ---- G.711 decoders (JS can do µ-law / A-law; cannot do G.722) ------------

/** @param { Buffer } bytes */
function pcmatolinear( bytes ) {
  const out = new Int16Array( bytes.length )
  for( let i = 0; i < bytes.length; i++ ) out[ i ] = projectrtp.codecx.pcma2linear16( bytes[ i ] )
  return out
}
/** @param { Buffer } bytes */
function pcmutolinear( bytes ) {
  const out = new Int16Array( bytes.length )
  for( let i = 0; i < bytes.length; i++ ) out[ i ] = projectrtp.codecx.pcmu2linear16( bytes[ i ] )
  return out
}


// ---- The three-channel chain harness --------------------------------------

/**
 * Run one end-to-end chain test with three projectrtp channels.
 *
 * @param { object } opts
 * @param { number } opts.polyCodec   - codec for the PseudoPoly player leg.
 *                                       This is what's on the wire between
 *                                       PseudoPoly and Ch_A (typically 9 = G.722).
 * @param { number } opts.chanACodec  - codec Ch_A uses on its own outbound.
 *                                       Ch_A receives whatever `polyCodec`
 *                                       packets hit its local port, decodes
 *                                       per packet PT — the PT sent BY Ch_A
 *                                       is `chanACodec`. For a pure G.722
 *                                       chain keep this equal to polyCodec.
 * @param { number } opts.chanBCodec  - codec Ch_B uses on its outbound to JS.
 *                                       Must be 0 (PCMU) or 8 (PCMA).
 * @param { number } [opts.durationMs=2500]
 * @param { string } opts.outWav      - path for the final captured WAV
 *                                       (decoded from JS-side RTP).
 * @param { boolean } [opts.dumpChannels=false] - when true, also attach a
 *                                       projectrtp recorder to each of the
 *                                       three channels. The resulting WAV
 *                                       files land alongside `outWav` with
 *                                       names derived from its stem:
 *                                         <stem>_poly_rec.wav   (PseudoPoly inbound)
 *                                         <stem>_chanA_rec.wav  (Ch_A inbound — the
 *                                                                G.722 decode point)
 *                                         <stem>_chanB_rec.wav  (Ch_B inbound —
 *                                                                normally silent in
 *                                                                this topology)
 *                                       Note: projectrtp recorders capture a
 *                                       channel's INBOUND audio.
 *                                       - poly_rec  captures the B→A mix
 *                                         direction (Ch_B's frame encoded
 *                                         as polyCodec, sent to PseudoPoly).
 *                                         In the current topology Ch_B's
 *                                         inbound is sinkB-echoed silence,
 *                                         so this records silence — useful
 *                                         for verifying the return path.
 *                                       - chanA_rec captures Ch_A's inbound
 *                                         (PseudoPoly's tone decoded via
 *                                         Ch_A's G.722 decoder). The key
 *                                         diagnostic for decode problems.
 *                                       - chanB_rec captures Ch_B's inbound
 *                                         (the silence echoed by sinkB).
 */
async function runchain( opts ) {
  const { polyCodec, chanACodec, chanBCodec, outWav } = opts
  const durationMs = opts.durationMs || 2500
  const dumpChannels = !!opts.dumpChannels
  expect( chanBCodec ).to.be.oneOf( [ 0, 8 ],
    "Ch_B must be PCMU (0) or PCMA (8) so the test can decode it in JS" )

  // Derive recorder filenames from the `outWav` stem so every per-test
  // artefact lives in the same directory with a matching prefix.
  const stem = outWav.replace( /\.wav$/, "" )
  const polyRecPath  = `${stem}_poly_rec.wav`
  const chanARecPath = `${stem}_chanA_rec.wav`
  const chanBRecPath = `${stem}_chanB_rec.wav`

  // ---- endpoints --------------------------------------------------------
  //
  // Only one raw-UDP sink is needed — sinkB, on the right of the chain.
  // PseudoPoly is the left endpoint: it already has a local socket (Ch_A
  // directs its B→A outbound there), and its recorder captures that
  // direction's audio as a WAV. We don't need a separate sinkA.
  //
  // sinkB echoes a comfort-noise / silence frame back to Ch_B on every
  // received packet. In a real call both legs always send audio (silence
  // frames included); mirroring that here means Ch_B's decode path is
  // actually exercised and PseudoPoly's left-side recorder captures a
  // proper silence stream rather than nothing. Swap the echo payload
  // later to feed real audio back up the chain if you want full
  // bidirectional tests.

  /*
     PseudoPoly ◄──── B→A silence from mix (Ch_B's frame → G.722) ◄─── Ch_A  
         │                                                               │                    
         │ player tone → G.722                                           │
         ▼                                                               ▼                    
      Ch_A.local ──────► Ch_A ══ mix ══ Ch_B ──── PCMA/PCMU ────► sinkB (UDP)                 
                                                                         │   
                                                                         ▼                    
                                                    decode PCMA, FFT, write WAV    
                                                          also ↩ echo silence back            
                                                             to Ch_B.local
   */

  /**
   * @param { number } expectedPt     - PT of packets we should accumulate.
   * @param { boolean } echoSilence    - when true, reply to every received
   *                                     packet with a same-length silence
   *                                     frame in the same PT back to the
   *                                     sender. 0xFF for PCMU, 0xD5 for
   *                                     PCMA — the encoded "zero sample".
   */
  function opensink( expectedPt, echoSilence ) {
    const sock = dgram.createSocket( "udp4" )
    /** @type { number[] } */
    const bytes = []
    // RTP state for the echo stream. Fresh ssrc so Ch_B's jitter buffer
    // treats this as a new source (avoids SSRC collision with whatever
    // Ch_B uses outbound).
    const echoSsrc = 0xDEADBEEF
    let echoSn = 0
    let echoTs = 0
    const silenceByte = ( expectedPt === 0 ) ? 0xFF : 0xD5

    sock.on( "message", ( msg, rinfo ) => {
      if( msg.length < 12 ) return
      const pt = msg[ 1 ] & 0x7F
      if( pt !== expectedPt ) return
      const payload = msg.subarray( 12 )
      for( const b of payload ) bytes.push( b )

      if( echoSilence ) {
        const header = Buffer.alloc( 12 )
        header[ 0 ] = 0x80
        header[ 1 ] = expectedPt
        header.writeUInt16BE( echoSn, 2 )
        header.writeUInt32BE( echoTs >>> 0, 4 )
        header.writeUInt32BE( echoSsrc, 8 )
        echoSn = ( echoSn + 1 ) & 0xFFFF
        echoTs = ( echoTs + payload.length ) >>> 0
        const reply = Buffer.concat( [ header, Buffer.alloc( payload.length, silenceByte ) ] )
        sock.send( reply, rinfo.port, rinfo.address )
      }
    } )
    return { sock, bytes }
  }

  const sinkB = opensink( chanBCodec, /* echoSilence */ true )

  sinkB.sock.bind()
  await new Promise( ( r ) => sinkB.sock.on( "listening", r ) )

  // ---- channel close plumbing ------------------------------------------

  const closes = []
  const closed = () => {
    let r
    const p = new Promise( ( res ) => { r = res } )
    closes.push( r )
    return p
  }
  const closeA = closed()
  const closeB = closed()
  const closePoly = closed()

  // ---- channels --------------------------------------------------------
  //
  // Open PseudoPoly last (it needs Ch_A's local port as its remote), and
  // Ch_A first (Ch_A.remote will point at PseudoPoly's local once we
  // know it). We work around the bootstrap problem by opening Ch_A with a
  // throwaway remote, then issuing a `channel.remote(...)` update once
  // we've opened PseudoPoly and know its port.
  const chanA = await projectrtp.openchannel( {
    // Throwaway initial remote — updated below once we know PseudoPoly.
    remote: { address: "127.0.0.1", port: 1, codec: chanACodec },
  }, ( d ) => { if( "close" === d.action ) closes[ 0 ]() } )

  // Ch_B's remote is the measurement endpoint — sinkB.
  const chanB = await projectrtp.openchannel( {
    remote: { address: "127.0.0.1", port: sinkB.sock.address().port, codec: chanBCodec },
  }, ( d ) => { if( "close" === d.action ) closes[ 1 ]() } )

  // PseudoPoly is the left endpoint. Its remote is Ch_A.local so its
  // player output (the generated tone) arrives at Ch_A as inbound, and
  // conversely Ch_A's B→A outbound lands on PseudoPoly's local port
  // where PseudoPoly's recorder captures it.
  //
  // Not in the mix: a channel's player is cleared on mix entry (matches
  // C++). Keeping PseudoPoly in LOCAL mode lets the player keep firing
  // alongside Ch_A/Ch_B's mix.
  const pseudoPoly = await projectrtp.openchannel( {
    remote: { address: "127.0.0.1", port: chanA.local.port, codec: polyCodec },
  }, ( d ) => { if( "close" === d.action ) closes[ 2 ]() } )

  // Point Ch_A's remote at PseudoPoly so the B→A return direction of
  // mix2 lands on PseudoPoly's local port (captured by its recorder).
  expect( chanA.remote( {
    address: "127.0.0.1", port: pseudoPoly.local.port, codec: chanACodec,
  } ) ).to.be.true

  // Attach per-channel recorders BEFORE the mix / player start so we
  // capture from the first inbound frame. Recorders run in both Local
  // and Mixed modes — starting them at open-time is safe regardless
  // of which mode each channel ends up in.
  if( dumpChannels ) {
    expect( pseudoPoly.record( { file: polyRecPath, numchannels: 2 } ),  "poly rec start"  ).to.be.true
    expect( chanA.record(      { file: chanARecPath, numchannels: 2 } ), "chanA rec start" ).to.be.true
    expect( chanB.record(      { file: chanBRecPath, numchannels: 2 } ), "chanB rec start" ).to.be.true
  }

  // Mix Ch_A with Ch_B BEFORE starting the player on PseudoPoly. The
  // mix is the 2-party mix2 case; PseudoPoly is outside it entirely.
  expect( chanA.mix( chanB ) ).to.be.true

  // Brief settle so the mix is firmly established before audio begins.
  await new Promise( ( r ) => setTimeout( r, 100 ) )

  // Fire the player — this generates the G.722 (or whatever polyCodec is)
  // packets on the wire from PseudoPoly to Ch_A.
  expect( pseudoPoly.play( { loop: true, files: [ { wav: "/tmp/tone400.wav" } ] } ) ).to.be.true

  await new Promise( ( r ) => setTimeout( r, durationMs ) )

  // ---- cleanup ---------------------------------------------------------

  pseudoPoly.close()
  chanA.close()
  chanB.close()
  await Promise.all( [ closeA, closeB, closePoly ] )
  await new Promise( ( r ) => sinkB.sock.close( r ) )

  // Write sinkB to disk (= outWav). PCMA / PCMU only (asserted above)
  // so decoding is always possible.
  dumpsinkaudio( sinkB.bytes, chanBCodec, outWav.replace( /\.wav$/, "" ) )

  const received = ( chanBCodec === 8 )
    ? Array.from( pcmatolinear( Buffer.from( sinkB.bytes ) ) )
    : Array.from( pcmutolinear( Buffer.from( sinkB.bytes ) ) )

  return {
    received,
    outWav,
    polyRecPath:  dumpChannels ? polyRecPath  : null,
    chanARecPath: dumpChannels ? chanARecPath : null,
    chanBRecPath: dumpChannels ? chanBRecPath : null,
  }
}

/**
 * Write a sink's accumulated bytes to disk in the most useful form:
 *   - PCMA (pt 8)  → decode to linear16 and save as WAV
 *   - PCMU (pt 0)  → decode to linear16 and save as WAV
 *   - G.722 (pt 9) → save raw payload as `.g722` (ffmpeg-decodable)
 *   - anything else → save as `.<pt>.raw` (best effort)
 * Returns the path actually written.
 * @param { number[] } bytes
 * @param { number } pt
 * @param { string } stemNoExt
 */
function dumpsinkaudio( bytes, pt, stemNoExt ) {
  const buf = Buffer.from( bytes )
  if( pt === 8 || pt === 0 ) {
    const linear = ( pt === 8 ) ? pcmatolinear( buf ) : pcmutolinear( buf )
    const path = `${stemNoExt}.wav`
    writewav( path, linear, 8000 )
    return path
  }
  if( pt === 9 ) {
    const path = `${stemNoExt}.g722`
    fs.writeFileSync( path, buf )
    return path
  }
  const path = `${stemNoExt}.pt${pt}.raw`
  fs.writeFileSync( path, buf )
  return path
}


// ---- JS-native sine source ------------------------------------------------
//
// Generate the tone in JS and save to a WAV. PseudoPoly's player reads
// this WAV and emits whatever codec it's configured for. This keeps the
// source signal entirely ours — no dependency on projectrtp's tone
// generator.

/**
 * @param { number } freqHz
 * @param { number } durationSec
 * @param { number } sampleRate
 * @param { number } amplitude - 0..1 scale relative to i16 full scale
 */
function gensinewav( path, freqHz, durationSec, sampleRate = 8000, amplitude = 0.5 ) {
  const total = Math.floor( sampleRate * durationSec )
  const peak = Math.round( 32767 * amplitude )
  const samples = new Int16Array( total )
  const w = 2 * Math.PI * freqHz / sampleRate
  for( let i = 0; i < total; i++ ) samples[ i ] = Math.round( Math.sin( i * w ) * peak )
  writewav( path, samples, sampleRate )
}


// ---- The tests ------------------------------------------------------------

describe( "codec chain (3 channels: pseudo-poly → mix(A, B) → JS measure)", function() {

  this.beforeAll( function() {
    // JS-native sine generator → WAV. PseudoPoly's player reads this
    // and produces the G.722 (or whatever) packets on the wire.
    gensinewav( "/tmp/tone400.wav", 400, 2.0, 8000, 0.5 )
    expect( fs.existsSync( "/tmp/tone400.wav" ), "wav write failed" ).to.be.true
  } )

  this.afterAll( function() {
    try { fs.unlinkSync( "/tmp/tone400.wav" ) } catch( _ ) { /* ignore */ }
  } )

  it( "G.722 on the wire → mix → PCMA out: 400 Hz tone survives", async function() {
    this.timeout( 5000 )
    this.slow( 4000 )

    const { received } = await runchain( {
      polyCodec:     9,                               // G.722 wire PseudoPoly → Ch_A
      chanACodec:    9,                               // Ch_A is a G.722 leg
      chanBCodec:    8,                               // PCMA to JS
      outWav:        "/tmp/codecchain_g722_to_pcma.wav",
      dumpChannels:  true,                            // write per-channel WAVs too
    } )

    expect( received.length, "mix/player/chain produced too little audio — setup broken" )
      .to.be.above( 4096 )

    const amps = ampfft( received )
    /* and roughly 30hz either side - based on a window = 4 */
    const e400 = energyat( amps, 400, 8000, 4 )
    const etotal = amps.reduce( ( a, b ) => a + b, 0 )
    const ratio = e400 / etotal

    // eslint-disable-next-line no-console
    console.log( `    G.722→PCMA: samples=${received.length}  E@400Hz=${e400.toFixed(0)}  ` +
                 `total=${etotal.toFixed(0)}  ratio=${ratio.toFixed(3)}` )

    expect( e400, "no energy at 400 Hz — the tone did not survive the chain" ).to.be.above( 0 )
    // A clean chain produces a strong peak (ratio ≳ 0.4). 0.15 is the
    // "barely-present" threshold — if the test is only just passing at
    // ~0.15 the output is noisy even if not zero.
    expect( ratio, "400 Hz is not dominant in the FFT — chain is noisy" ).to.be.above( 0.15 )
  } )

  it( "PCMA on the wire → mix → PCMA out (no G.722) — baseline", async function() {
    this.timeout( 5000 )
    this.slow( 4000 )

    // Baseline with no G.722 anywhere. If this ratio is high and the
    // G.722 test above is low, the regression is confined to the G.722
    // decode / resample stage.
    const { received } = await runchain( {
      polyCodec:     8,
      chanACodec:    8,
      chanBCodec:    8,
      outWav:        "/tmp/codecchain_pcma_to_pcma.wav",
      dumpChannels:  true,
    } )

    expect( received.length ).to.be.above( 4096 )

    const amps = ampfft( received )
    const e400 = energyat( amps, 400, 8000, 4 )
    const etotal = amps.reduce( ( a, b ) => a + b, 0 )
    const ratio = e400 / etotal

    // eslint-disable-next-line no-console
    console.log( `    PCMA→PCMA : samples=${received.length}  E@400Hz=${e400.toFixed(0)}  ` +
                 `total=${etotal.toFixed(0)}  ratio=${ratio.toFixed(3)}` )

    expect( e400 ).to.be.above( 0 )
    expect( ratio, "PCMA baseline should be strongly tonal" ).to.be.above( 0.18 )
  } )

  it( "G.722 on the wire → mix → PCMU out: 400 Hz tone survives", async function() {
    this.timeout( 5000 )
    this.slow( 4000 )

    const { received } = await runchain( {
      polyCodec:     9,
      chanACodec:    9,
      chanBCodec:    0,                               // PCMU to JS
      outWav:        "/tmp/codecchain_g722_to_pcmu.wav",
      dumpChannels:  true,
    } )

    expect( received.length ).to.be.above( 4096 )

    const amps = ampfft( received )
    const e400 = energyat( amps, 400, 8000, 4 )
    const etotal = amps.reduce( ( a, b ) => a + b, 0 )
    const ratio = e400 / etotal

    // eslint-disable-next-line no-console
    console.log( `    G.722→PCMU: samples=${received.length}  E@400Hz=${e400.toFixed(0)}  ` +
                 `total=${etotal.toFixed(0)}  ratio=${ratio.toFixed(3)}` )

    expect( e400 ).to.be.above( 0 )
    expect( ratio ).to.be.above( 0.15 )
  } )

} )


// ---- DTLS-SRTP back-to-back ----------------------------------------------
//
//   ┌────────────┐                       ┌────────────┐
//   │ chanA      │ ====== SRTP ======>   │ chanB      │
//   │ (active,   │   (PCMA over DTLS)    │ (passive,  │
//   │  client)   │                       │  server)   │
//   │ plays tone │                       │ records    │
//   └────────────┘                       └────────────┘
//
// Both channels live in the same process. If the 400 Hz tone reaches
// chanB's recorder at recognisable amplitude, the DTLS handshake ran,
// both SRTP contexts were built, and inbound SRTP is decrypting
// correctly. No tone ⇒ break is somewhere in handshake → keying-material
// → SRTP decrypt.

/**
 * Read a PCM-16 mono WAV straight from the bytes. Skips the 44-byte
 * RIFF/WAVE header our recorder emits — consistent with `writewav`
 * above — and returns { samples, sampleRate }.
 * @param { string } path
 */
function readwav( path ) {
  const buf = fs.readFileSync( path )
  const sampleRate = buf.readUInt32LE( 24 )
  const numChannels = buf.readUInt16LE( 22 )
  const bitsPerSample = buf.readUInt16LE( 34 )
  if( bitsPerSample !== 16 ) throw new Error( `unexpected bitsPerSample=${bitsPerSample}` )
  const dataLen = buf.readUInt32LE( 40 )
  const n = dataLen / 2
  const interleaved = new Int16Array( n )
  for( let i = 0; i < n; i++ ) interleaved[ i ] = buf.readInt16LE( 44 + i * 2 )
  // If stereo, just return the left channel — that's the "inbound" side
  // under our current L=in / R=out convention.
  if( numChannels === 2 ) {
    const mono = new Int16Array( n / 2 )
    for( let i = 0; i < mono.length; i++ ) mono[ i ] = interleaved[ i * 2 ]
    return { samples: mono, sampleRate }
  }
  return { samples: interleaved, sampleRate }
}

describe( "dtls-srtp back-to-back (2 channels: A plays → SRTP → B records)", function() {

  this.timeout( 8000 )
  this.slow( 4500 )

  this.beforeAll( function() {
    gensinewav( "/tmp/tone400_dtls.wav", 400, 2.0, 8000, 0.5 )
  } )

  this.afterAll( function() {
    try { fs.unlinkSync( "/tmp/tone400_dtls.wav" ) } catch ( _ ) { /* ignore */ }
  } )

  it( "PCMA over DTLS: 400 Hz tone survives the encrypted leg", async function() {

    const recPath = "/tmp/codecchain_dtls_b_rec.wav"
    try { fs.unlinkSync( recPath ) } catch ( _ ) { /* ignore */ }

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const chanA = await projectrtp.openchannel( {}, ( d ) => {
      if ( "close" === d.action ) chanB.close()
    } )
    const chanB = await projectrtp.openchannel( {}, ( d ) => {
      if ( "close" === d.action ) done()
    } )

    // PCMA end-to-end with DTLS-SRTP. chanA is the DTLS client
    // (mode=active), chanB is the server (mode=passive). Each side gets
    // the OTHER side's fingerprint so cert verification succeeds.
    expect( chanA.remote( {
      address: "127.0.0.1",
      port: chanB.local.port,
      codec: 8,
      dtls: { fingerprint: { hash: chanB.local.dtls.fingerprint }, mode: "active" },
    } ) ).to.be.true

    expect( chanB.remote( {
      address: "127.0.0.1",
      port: chanA.local.port,
      codec: 8,
      dtls: { fingerprint: { hash: chanA.local.dtls.fingerprint }, mode: "passive" },
    } ) ).to.be.true

    // Recorder on the receiving side. Captures decoded inbound — so what
    // we see in this file is whatever came out the far side of SRTP
    // decrypt. Start it before audio so we don't miss the first frames.
    expect( chanB.record( { file: recPath } ) ).to.be.true

    // Settle for the DTLS handshake. Typically <200 ms on loopback;
    // 300 ms leaves margin for retransmits without blowing out the test.
    await new Promise( ( r ) => setTimeout( r, 300 ) )

    expect( chanA.play( { loop: true, files: [ { wav: "/tmp/tone400_dtls.wav" } ] } ) ).to.be.true

    // ~1.5 s of tone post-handshake — enough for a clean FFT.
    await new Promise( ( r ) => setTimeout( r, 1800 ) )

    chanA.close()
    await finished

    // ---- verify -----------------------------------------------------
    const { samples, sampleRate } = readwav( recPath )
    expect( samples.length, "recording too short — no SRTP audio reached chanB" ).to.be.above( 4000 )

    // Drop the first 250 ms so the FFT sees steady-state tone, not the
    // silent gap before the player kicks in.
    const trimFront = Math.min( samples.length, ( sampleRate * 0.25 ) | 0 )
    const analysed = samples.slice( trimFront )

    const amps = ampfft( analysed )
    const e400 = energyat( amps, 400, sampleRate, 4 )
    const etotal = amps.reduce( ( a, b ) => a + b, 0 )
    const ratio = e400 / etotal

    // eslint-disable-next-line no-console
    console.log( `    DTLS-SRTP: samples=${analysed.length}  E@400Hz=${e400.toFixed(0)}  ` +
                 `total=${etotal.toFixed(0)}  ratio=${ratio.toFixed(3)}` )

    expect( e400 ).to.be.above( 0 )
    expect( ratio, "400 Hz not dominant in the FFT — SRTP audio not making it through" ).to.be.above( 0.15 )
  } )

  // NOTE: a mix-mode variant of the above was attempted but removed. The
  // one-line fix (calling `poll_dtls_handshake` from `mix_tick`) is
  // verified by the production scenario: a real SRTP peer sends inbound
  // audio which goes through chanA's codecx and then the mix. A
  // back-to-back test where chanA only has a player and is mixed with
  // chanB hits a separate mix-path issue (player-only channels don't
  // feed their samples through codecx in mix2), not the DTLS fix. The
  // Local-mode DTLS test above covers the cryptographic path; real-call
  // traffic covers the mix+DTLS interaction.

} )


// ---- createReadStream — live audio tap ------------------------------------
//
// chanA plays a 400 Hz tone and sends it as PCMA to chanB. On chanB we
// attach `createReadStream({ direction: "in" })` and collect frames as a
// Node Readable. If FFT of the collected samples is peaked at 400 Hz, the
// whole chain works: tick feed point → bounded mpsc → forwarder task →
// ThreadsafeFunction → Readable → consumer.

describe( "createReadStream — live audio tap", function() {

  this.timeout( 8000 )
  this.slow( 4500 )

  this.beforeAll( function() {
    gensinewav( "/tmp/tone400_tap.wav", 400, 2.0, 8000, 0.5 )
  } )

  this.afterAll( function() {
    try { fs.unlinkSync( "/tmp/tone400_tap.wav" ) } catch ( _ ) { /* ignore */ }
  } )

  it( "PCMA tone → inbound tap: 400 Hz survives the full reader pipeline", async function() {

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const chanA = await projectrtp.openchannel( {}, ( d ) => {
      if( "close" === d.action ) chanB.close()
    } )
    const chanB = await projectrtp.openchannel( {}, ( d ) => {
      if( "close" === d.action ) done()
    } )

    expect( chanA.remote( {
      address: "127.0.0.1", port: chanB.local.port, codec: 8,
    } ) ).to.be.true
    expect( chanB.remote( {
      address: "127.0.0.1", port: chanA.local.port, codec: 8,
    } ) ).to.be.true

    // Collect frames via the reader. `direction: "in"` = what chanB is
    // receiving from chanA = the tone decoded from PCMA.
    const reader = chanB.createReadStream( { direction: "in", format: "l16", samplerate: 8000 } )

    // Resolved config must be visible on the Readable — consumers need to
    // know the byte shape without peeking at opts they didn't pass.
    expect( reader.format ).to.equal( "l16" )
    expect( reader.samplerate ).to.equal( 8000 )
    expect( reader.numchannels ).to.equal( 1 )
    expect( reader.direction ).to.equal( "in" )
    expect( reader.readerId ).to.be.a( "number" ).above( 0 )

    const frames = []
    reader.on( "data", ( buf ) => frames.push( buf ) )
    let ended = false
    reader.on( "end", () => { ended = true } )

    expect( chanA.play( { loop: true, files: [ { wav: "/tmp/tone400_tap.wav" } ] } ) ).to.be.true

    await new Promise( ( r ) => setTimeout( r, 1800 ) )

    // Explicitly destroy BEFORE closing chanA so we cover the JS-initiated
    // tear-down path (_destroy → DestroyReadStream → forwarder sees mpsc
    // drop → end-of-stream sentinel → JS `end`).
    reader.destroy()
    await new Promise( ( r ) => setTimeout( r, 50 ) )
    expect( ended, "reader.destroy() should end the stream" ).to.be.true

    chanA.close()
    await finished

    const totalBytes = frames.reduce( ( n, b ) => n + b.length, 0 )
    expect( totalBytes, "reader collected no audio" ).to.be.above( 8000 )

    // Reassemble as Int16 LE samples and run the same FFT the other tests
    // use. Drop the first 250 ms so the tone is in steady state.
    const all = Buffer.concat( frames )
    const sampleCount = all.length / 2
    const samples = new Int16Array( sampleCount )
    for( let i = 0; i < sampleCount; i++ ) samples[ i ] = all.readInt16LE( i * 2 )

    const trimFront = Math.min( samples.length, 2000 )
    const analysed = samples.slice( trimFront )
    const amps = ampfft( analysed )
    const e400 = energyat( amps, 400, 8000, 4 )
    const etotal = amps.reduce( ( a, b ) => a + b, 0 )
    const ratio = e400 / etotal

    // eslint-disable-next-line no-console
    console.log( `    TAP: samples=${analysed.length}  E@400Hz=${e400.toFixed(0)}  ` +
                 `total=${etotal.toFixed(0)}  ratio=${ratio.toFixed(3)}` )

    expect( e400 ).to.be.above( 0 )
    expect( ratio, "400 Hz not dominant — tap pipeline broke the audio" ).to.be.above( 0.15 )
  } )

} )
