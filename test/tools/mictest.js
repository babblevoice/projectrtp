#!/usr/bin/env node

/**
 * mictest.js — Manual test tool for playrecord using local microphone and speakers.
 *
 * Starts projectrtp, opens a channel, and runs a playrecord command that:
 *   1. Plays a prompt WAV file through your speakers
 *   2. Records your microphone response to a WAV file
 *
 * Requires sox installed: `sudo apt install sox` / `brew install sox` / `apk add sox`
 *
 * Usage:
 *   node test/tools/mictest.js [options]
 *
 * Options:
 *   --prompt <file>       WAV file to play as prompt (default: generates a test tone)
 *   --output <file>       WAV file to save recording (default: /tmp/mictest_recording.wav)
 *   --duration <ms>       Max recording duration in ms (default: 10000)
 *   --interrupt           Enable barge-in (speech interrupts prompt playback)
 *   --bargeinpower <n>    Power threshold for barge-in (default: 500)
 *
 * Examples:
 *   node test/tools/mictest.js
 *   node test/tools/mictest.js --prompt /tmp/myprompt.wav --interrupt
 *   node test/tools/mictest.js --output /tmp/myrecording.wav --duration 5000
 */

const dgram = require( "dgram" )
const { spawn } = require( "child_process" )
const prtp = require( "../../index.js" )

/* parse args */
const args = process.argv.slice( 2 )
function getarg( name, defaultval ) {
  const idx = args.indexOf( name )
  if( idx === -1 ) return defaultval
  return args[ idx + 1 ] || defaultval
}

const promptfile = getarg( "--prompt", "" )
const outputfile = getarg( "--output", "/tmp/mictest_recording.wav" )
const maxduration = parseInt( getarg( "--duration", "10000" ) )
const interrupt = args.includes( "--interrupt" )
const bargeinpower = parseInt( getarg( "--bargeinpower", "10" ) )

/* generate a test tone if no prompt file given */
const fs = require( "fs" )
function generatetesttone( filepath ) {
  const samplerate = 8000
  const duration = 1.5
  const samples = samplerate * duration
  const bytespersample = 2

  const header = Buffer.alloc( 44 )
  header.write( "RIFF", 0 )
  header.writeUInt32LE( 36 + samples * bytespersample, 4 )
  header.write( "WAVE", 8 )
  header.write( "fmt ", 12 )
  header.writeUInt32LE( 16, 16 )
  header.writeUInt16LE( 1, 20 )
  header.writeUInt16LE( 1, 22 )
  header.writeUInt32LE( samplerate, 24 )
  header.writeUInt32LE( samplerate * bytespersample, 28 )
  header.writeUInt16LE( bytespersample, 32 )
  header.writeUInt16LE( 16, 34 )
  header.write( "data", 36 )
  header.writeUInt32LE( samples * bytespersample, 40 )

  const data = Buffer.alloc( samples * bytespersample )
  for( let i = 0; i < samples; i++ ) {
    /* beep tone: 800Hz for 0.3s, silence, 800Hz for 0.3s, then silence */
    const t = i / samplerate
    let val = 0
    if( t < 0.3 || ( t > 0.5 && t < 0.8 ) ) {
      val = Math.sin( i * Math.PI * 2 * 800 / samplerate ) * 15000
    }
    data.writeInt16LE( Math.round( val ), i * 2 )
  }

  fs.writeFileSync( filepath, Buffer.concat( [ header, data ] ) )
  return filepath
}

async function main() {
  const wavfile = promptfile || generatetesttone( "/tmp/mictest_prompt.wav" )

  if( !fs.existsSync( wavfile ) ) {
    console.error( `Prompt file not found: ${wavfile}` )
    process.exit( 1 )
  }

  console.log( "Starting projectrtp..." )
  prtp.projectrtp.run()

  /* create UDP socket to simulate the remote RTP endpoint */
  const udp = dgram.createSocket( "udp4" )
  udp.bind()
  await new Promise( r => udp.on( "listening", r ) )

  const localport = udp.address().port
  let channelport

  /* start sox for speaker output — RTP payloads are mu-law, sox decodes to speakers */
  let speaker
  try {
    speaker = spawn( "sox", [
      "-t", "raw", "-r", "8000", "-c", "1", "-e", "mu-law", "-b", "8", "-",
      "-d"
    ], { stdio: [ "pipe", "inherit", "inherit" ] } )
    speaker.on( "error", () => {
      console.error( "Warning: sox not available for speaker output" )
      speaker = null
    } )
  } catch( e ) {
    console.error( "Warning: could not start speaker output" )
    speaker = null
  }

  /* start sox for microphone capture — capture as mu-law so it can be sent directly as PCMU RTP */
  let mic
  try {
    mic = spawn( "sox", [
      "-d",
      "-t", "raw", "-r", "8000", "-c", "1", "-e", "mu-law", "-b", "8", "-"
    ], { stdio: [ "inherit", "pipe", "inherit" ] } )
    mic.on( "error", () => {
      console.error( "Warning: sox not available for mic capture" )
      mic = null
    } )
  } catch( e ) {
    console.error( "Warning: could not start mic capture" )
    mic = null
  }

  /* RTP packet helpers */
  let sn = 100
  let ts = 0
  const ssrc = 12345

  function makertppacket( payload ) {
    const header = Buffer.alloc( 12 )
    header[ 0 ] = 0x80
    header[ 1 ] = 0x00 /* PCMU */
    header.writeUInt16BE( sn % 65536, 2 )
    header.writeUInt32BE( ts, 4 )
    header.writeUInt32BE( ssrc, 8 )
    sn++
    ts += 160
    return Buffer.concat( [ header, payload ] )
  }

  /* receive RTP from channel → play through speakers */
  udp.on( "message", ( msg ) => {
    if( msg.length < 12 ) return
    const payload = msg.subarray( 12 )
    if( speaker && speaker.stdin.writable ) {
      speaker.stdin.write( payload )
    }
  } )

  /* capture mic → send as RTP to channel */
  let micbuffer = Buffer.alloc( 0 )
  if( mic ) {
    mic.stdout.on( "data", ( data ) => {
      micbuffer = Buffer.concat( [ micbuffer, data ] )
      /* send in 160-byte (20ms) chunks */
      while( micbuffer.length >= 160 ) {
        const chunk = micbuffer.subarray( 0, 160 )
        micbuffer = micbuffer.subarray( 160 )
        const pkt = makertppacket( chunk )
        if( channelport ) {
          udp.send( pkt, channelport, "127.0.0.1" )
        }
      }
    } )
  }

  console.log( `Prompt: ${wavfile}` )
  console.log( `Output: ${outputfile}` )
  console.log( `Interrupt: ${interrupt}` )
  console.log( `Max duration: ${maxduration}ms` )
  console.log( "" )

  /* open channel */
  let done
  const finished = new Promise( r => done = r )

  const channel = await prtp.projectrtp.openchannel(
    { "remote": { "address": "127.0.0.1", "port": localport, "codec": 0 } },
    function( d ) {
      const timestamp = new Date().toISOString().substr( 11, 12 )
      console.log( `[${timestamp}] Event: ${d.action} ${d.event || ""} ${d.reason || ""} ${d.file || ""}`.trim() )

      if( "record" === d.action && d.event && d.event.startsWith( "finished" ) ) {
        console.log( "\nRecording finished." )
        if( fs.existsSync( outputfile ) ) {
          const stats = fs.statSync( outputfile )
          console.log( `Output: ${outputfile} (${stats.size} bytes)` )
          const info = prtp.projectrtp.soundfile.info( outputfile )
          console.log( `WAV: ${info.samplerate}Hz, ${info.channelcount}ch, ${info.bitdepth}bit` )
        }
        setTimeout( () => {
          channel.close()
        }, 200 )
      }

      if( "close" === d.action ) {
        done()
      }
    }
  )

  channelport = channel.local.port

  /* send silence packets at 20ms intervals to keep the RTP stream alive
     until the mic capture takes over. stops when mic starts producing data. */
  const silence = Buffer.alloc( 160, 0xff ) /* PCMU silence = 0xff */
  let micstarted = false
  const silencetimer = setInterval( () => {
    if( micstarted ) return
    const pkt = makertppacket( silence )
    udp.send( pkt, channelport, "127.0.0.1" )
  }, 20 )

  if( mic ) {
    mic.stdout.once( "data", () => { micstarted = true } )
  }

  /* send a few packets immediately so the channel learns our address */
  for( let i = 0; i < 5; i++ ) {
    const pkt = makertppacket( silence )
    udp.send( pkt, channelport, "127.0.0.1" )
    await new Promise( r => setTimeout( r, 20 ) )
  }

  console.log( "Playing prompt and recording... speak after the beep!\n" )

  /* issue playrecord command */
  const playrecordopts = {
    "soup": { "files": [ { "wav": wavfile } ] },
    "record": {
      "file": outputfile,
      "numchannels": 1,
      "finishbelowpower": 80,
      "minduration": 1000,
      "maxduration": maxduration,
      "poweraveragepackets": 30
    }
  }

  if( interrupt ) {
    playrecordopts.interrupt = true
    playrecordopts.bargeinpower = bargeinpower
    playrecordopts.bargeinpoweraveragepackets = 5
  }

  channel.playrecord( playrecordopts )

  await finished

  /* cleanup */
  clearInterval( silencetimer )
  if( mic ) mic.kill()
  if( speaker ) {
    speaker.stdin.end()
    speaker.kill()
  }
  udp.close()

  console.log( "\nDone." )
  process.exit( 0 )
}

main().catch( e => {
  console.error( e )
  process.exit( 1 )
} )
