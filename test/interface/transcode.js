/* if we want to see what is going on - use nodeplotlib instead of our placeholder */
//const npl = require( "nodeplotlib" )
// eslint-disable-next-line no-unused-vars
const npl = { plot: ( /** @type {any} */ a ) => {} }


const fft = require( "fft-js" ).fft
const projectrtp = require( "../../index" ).projectrtp
const expect = require( "chai" ).expect
const dgram = require( "dgram" )
const fs = require( "fs" )
const pcap = require( "./pcap" )


/*
So that we do not have to impliment g722 or other codecs in JS, we create 2 channels, and mix them. 
On one end, we UDP echo back - which means, for example, g722 will be echoed back, then on the other end, 
we generate a signal (tone) and check we receive that signal on the end end.
*/

const datalength = 8192 /* 1 second of data */
const frequency = 400


const magnitude = ( Math.pow( 2, 16 ) / 2 ) - ( 65536 / 4 )

/**
 * Generates time series signal with one sinewave component @ hz
 * @param { number } hz 
 * @returns { Int16Array }
 */
function gensignal( hz ) {

  const y = new Int16Array( datalength )

  for( let i = 0; i < datalength; i ++ ) {
    y[ i ] = Math.sin( i * ( Math.PI * 2 * ( 1 / 8000 ) ) * hz ) * magnitude
  }

  /*
  npl.plot( [ {
    y: Array.from( y ),
    type: "scatter"
  } ] )
  */

  return y
}

/**
 * 
 * @param { Array } arr 
 * @returns 
 */
function truncatetopoweroftwo( arr ) {
  const newsize = Math.pow( 2, Math.floor( Math.log2( arr.length ) ) )
  return arr.slice( 0, newsize )
}

/**
 * 
 * @param { Array< Array< number > > } c - array of complex numbers as returned by fft 
 * @returns { Array< number > }
 */
function amplitude( c ) {
  const out = []

  for( let k = 0; k < c.length; k++ ) {
    const complex = c[ k ]
    const r = complex[ 0 ]
    const i = complex[ 1 ]
    out.push( Math.sqrt( ( r * r ) + ( i * i ) ) )
  }

  return out
}

/**
 * 
 * @param { Array< number > } inarr
 * @param { number } startpos 
 * @param { number } endpos 
 */
function sum( inarr, startpos, endpos ) {
  let oursum = 0
  for( let i = startpos; i < endpos; i++ ) oursum += inarr[ i ]
  return oursum
}

/**
 * 
 * @param { Int16Array } signal 
 * @returns { Array< number > }
 */
function ampbyfrequency( signal ) {
  const pow2signal = truncatetopoweroftwo( Array.from( signal ) )
  const ourfft = fft( pow2signal )
  const amps = amplitude( ourfft )

  /*
  npl.plot( [ {
    y: amps
  } ] )
  */

  return amps
}

/**
 * Checks fft of signal to see if we have a signal at hz present
 * @param { Array< number > } amps
 * @param { number } hz 
 * @param { number } threshold
 */
function has( amps , hz, threshold ) {
  return sum( amps, hz - 20, hz + 20 ) > threshold
}

/**
 * 
 * @param { Array< number > } inarray
 * @returns { Array< number > }
 */
function lineartopcma( inarray ) {
  const out = []
  for( let i = 0; i < inarray.length; i++ )
    out.push( projectrtp.codecx.linear162pcma( inarray[ i ] ) )
  return out
}

/**
 * 
 * @param { Array< number > } inarray
 * @returns { Int16Array }
 */
function pcmatolinear( inarray ) {
  const out = new Int16Array( inarray.length )
  for( let i = 0; i < inarray.length; i++ ) {
    out[ i ] = projectrtp.codecx.pcma2linear16( inarray[ i ] )
  }

  return out
}

/**
 * 
 * @param { Array< number > } inarray
 * @returns { Array< number > }
 */
function lineartopcmu( inarray ) {
  const out = []
  for( let i = 0; i < inarray.length; i++ )
    out.push( projectrtp.codecx.linear162pcmu( inarray[ i ] ) )
  return out
}

/**
 * 
 * @param { Array< number > } inarray
 * @returns { Int16Array }
 */
function pcmutolinear( inarray ) {
  const out = new Int16Array( inarray.length )
  for( let i = 0; i < inarray.length; i++ ) {
    out[ i ] = projectrtp.codecx.pcmu2linear16( inarray[ i ] )
  }

  return out
}

/**
 * Send Buffer to server at required time
 * @param { number } sendtime 
 * @param { Buffer } pk 
 * @param { number } dstport 
 * @param { dgram.Socket } server 
 * @returns 
 */
function sendpayload( sendtime, pk, dstport, server ) {
  return setTimeout( () => {
    server.send( pk, dstport, "localhost" )
  }, sendtime )
}

/**
 * 
 * @param { number } sn - should start from 0 which we use to index into the supplied data buffer
 * @param { number } dstport 
 * @param { object } server 
 * @param { number } pt - payload type
 * @param { number } ssrc  - a unique payload type
 * @param { Array< number > } payload
 * @param { number } [ snoffset = 0 ] - if we want to have an offset
 * @param { function } [ cb ] - callback when sent
 * @returns 
 */
function sendpk( sn, dstport, server, pt = 0, ssrc, payload, snoffset=0, cb ) {

  if( !ssrc ) ssrc = 25
  const ts = sn * 160
  const sendtime = sn * 20

  const uint8pl = new Uint8Array( payload.slice( sn , sn + 160 ) )

  return setTimeout( () => {
    const subheader = Buffer.alloc( 10 )

    subheader.writeUInt16BE( ( sn + snoffset ) % ( 2**16 ) )
    subheader.writeUInt32BE( ts, 2 )
    subheader.writeUInt32BE( ssrc, 6 )

    const rtppacket = Buffer.concat( [
      Buffer.from( [ 0x80, pt ] ),
      subheader,
      uint8pl ] )

    server.send( rtppacket, dstport, "localhost" )
    if( cb ) cb( { rtppacket, dstport } )
  }, sendtime )
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
 * @callback encodefunction
 * @param { Array< number > } inarray
 * @returns { Array< number > }
 */

/**
 * @callback decodefunction
 * @param { Array< number > } inarray
 * @returns { Int16Array }
 */

/**
 * Run a loop test: generate signal - endcode pass to a channel, mix with second channal
 * receive this rtp and loop back and finally recieve and test for signal in sound.
 * This tests the full audio loop with codec conversion.
 * The encode and decode functions must match the bcodec, i.e. bcodec tells 
 * projectrtp what codec to accept on that channel, the functions are what takes
 * our linear16 and encodes and decodes into the payload.
 * @param { number } acodec 
 * @param { number } bcodec 
 * @param { encodefunction } encode 
 * @param { decodefunction } decode
 * @param { number } [ ilbcpt = -1 ] if acodec is ilbc then set the dynamic pt
 */
async function looptest( acodec, bcodec, encode, decode, ilbcpt = -1 ) {
  const a = dgram.createSocket( "udp4" )
  const b = dgram.createSocket( "udp4" )

  a.bind()
  await new Promise( resolve => a.on( "listening", resolve ) )
  b.bind()
  await new Promise( resolve => b.on( "listening", resolve ) )

  let done
  const finished = new Promise( ( r ) => { done = r } )

  const channeladef = { "id": "4", "remote": { "address": "localhost", "port": a.address().port, "codec": acodec } }
  if( 97 == acodec && -1 != ilbcpt ) channeladef.remote.ilbcpt = ilbcpt

  const achannel = await projectrtp.openchannel( channeladef, function( d ) {
    if( "close" === d.action ) {
      a.close()
      b.close()
      bchannel.close()
    }
  } )

  const bchannel = await projectrtp.openchannel( { "id": "4", "remote": { "address": "localhost", "port": b.address().port, "codec": bcodec } }, function( d ) {
    if( "close" === d.action ) done()
  } )

  bchannel.mix( achannel )

  /* echo straight back */
  a.on( "message", function( msg ) {
    const rtppk = parsepk( msg )
    if( -1 != ilbcpt ) expect( rtppk.pt ).to.equal( ilbcpt )
    a.send( msg, achannel.local.port, "localhost" )
  } )

  let received = Buffer.alloc( 0 )
  let ondonereceiving, recvcount = 0
  const receiveuntil = new Promise( resolve => ondonereceiving = resolve )
  b.on( "message", function( msg ) {
    const pk = parsepk( msg )
    received = Buffer.concat( [ received, pk.payload ] )
    if( 50 < recvcount++ ) ondonereceiving()
  } )

  const y = gensignal( frequency )
  const encoded = encode( Array.from( y ) )

  for( let i = 0; 60 > i; i ++ ) {
    sendpk( i, bchannel.local.port, b, bcodec, 44, encoded )
  }

  await receiveuntil
  const y2 = decode( Array.from( received ) )

  achannel.close()
  await finished

  npl.plot( [ {
    y: Array.from( y ),
    type: "scatter"
  } ] )

  npl.plot( [ {
    y: Array.from( y2 ),
    type: "scatter"
  } ] )

  const amps = ampbyfrequency( y2 )
  expect( has( amps, frequency - 100, 25000000 ) ).to.be.false
  expect( has( amps, frequency, 25000000 ) ).to.be.true
  expect( has( amps, frequency + 100, 25000000 ) ).to.be.false
}

/**
 * Test to check we receive all packets. ALso check basic codecs to make sure
 * no duff packets come through (i,.e. memory is cleared out). It will only work
 * with non-lossy CODECS
 * @param { number } acodec 
 * @param { number } bcodec 
 * @param { encodefunction } encode 
 * @param { decodefunction } decode
 * @param { number } [ expectedval ] what value we expect after the round trip (i.e. ulaw - alawy and back again might not be the same value)
 */
async function loopcounttest( acodec, bcodec, encode, decode, expectedval = 0 ) {

  const a = dgram.createSocket( "udp4" )
  const b = dgram.createSocket( "udp4" )

  a.bind()
  await new Promise( resolve => a.on( "listening", resolve ) )
  b.bind()
  await new Promise( resolve => b.on( "listening", resolve ) )

  let done
  const finished = new Promise( ( r ) => { done = r } )

  const allstats = {
    a: {
      recv:{ count: 0 },
      send:{ count: 0 },
      port: a.address().port
    },
    b: {
      recv:{ count: 0 },
      send:{ count: 0 },
      srcport: b.address().port,
      dstport: 0
    },
    notcorrect: 0
  }

  const achannel = await projectrtp.openchannel( { "id": "4", "remote": { "address": "localhost", "port": a.address().port, "codec": acodec } }, function( d ) {
    if( "close" === d.action ) {
      a.close()
      b.close()
      bchannel.close()
      allstats.achannel = { stats: d.stats }
    }
  } )

  const bchannel = await projectrtp.openchannel( { "id": "4", "remote": { "address": "localhost", "port": b.address().port, "codec": bcodec } }, function( d ) {
    if( "close" === d.action ) {
      allstats.bchannel = { stats: d.stats }
      done()
    }
  } )

  allstats.b.dstport = bchannel.local.port

  /* echo straight back */
  a.on( "message", function( msg ) {
    a.send( msg, achannel.local.port, "localhost" )
    allstats.a.recv.count++
    allstats.a.send.count++
  } )

  bchannel.mix( achannel )

  b.on( "message", function( msg ) {
    allstats.b.recv.count++
    const pk = parsepk( msg )
    const decoded = decode( Array.from( pk.payload ) )
    for( let i = 0; i < decoded.length; i++ ) {
      if( expectedval != decoded[ i ] ) allstats.notcorrect++
    }
  } )

  const y = new Int16Array( datalength ).fill( 0 )
  const encoded = encode( Array.from( y ) )

  for( let i = 0; 60 > i; i ++ ) {
    sendpk( i, bchannel.local.port, b, bcodec, 44, encoded, 0, () => { allstats.b.send.count++ } )
  }

  const bufferdelay = 350
  const errormarin = 500
  const packettime = 20 * 60
  const totaltimerequired = packettime + bufferdelay + errormarin
  await new Promise( resolve => setTimeout( resolve, totaltimerequired ) )

  achannel.close()
  await finished

  return allstats
}



describe( "Transcode", function() {

  this.slow( 3000 )
  this.timeout( 5000 )

  it( "basic count test and data check trancode pcmu <==> pcmu", async function() {
    /* 2 seconds is important, it should be below 60 * 20mS + JT = 1200 + 300 + 300 = 1800mS - we are taking 1820 */
    this.timeout( 4000 )
    this.slow( 2000 )

    const all = []
    for( let i = 0; 50 > i; i++) {
      all.push( loopcounttest( 0, 0, lineartopcmu, pcmutolinear ) )
    }
    const results = await Promise.all( all )
    results.forEach( ( i ) => {
      expect( i.a.recv.count ).to.equal( 60 )
      expect( i.b.recv.count ).to.equal( 60 )
    } )
  } )

  it( "basic count test and data check trancode pcma <==> pcma", async function() {
    this.timeout( 3000 )
    this.slow( 2500 )
    const result = await loopcounttest( 0, 0, lineartopcma, pcmatolinear, 8 )
    expect( result.notcorrect ).to.equal( 0 )
  } )

  it( "basic count test and data check trancode pcmu <==> pcma", async function() {
    this.timeout( 3000 )
    this.slow( 2500 )
    const result = await loopcounttest( 8, 0, lineartopcmu, pcmutolinear, 8 )
    expect( result.notcorrect ).to.equal( 0 )
  } )

  it( "basic count test and data check trancode pcma <==> pcmu", async function() {
    this.timeout( 3000 )
    this.slow( 2500 )
    const result = await loopcounttest( 0, 8, lineartopcma, pcmatolinear, 8 )
    expect( result.notcorrect ).to.equal( 0 )
  } )

  it( "Test our linear to pcma converting routines", async function() {

    const y = gensignal( frequency )

    const pcma = lineartopcma( Array.from( y ) )
    const y2 = pcmatolinear( pcma )

    npl.plot( [ {
      y: y2,
      type: "scatter"
    } ] )

    const amps = ampbyfrequency( y )
    expect( has( amps, 300, 25000000 ) ).to.be.false
    expect( has( amps, 400, 25000000 ) ).to.be.true
    expect( has( amps, 500, 25000000 ) ).to.be.false
  } )

  it( "trancode pcmu <==> ilbc static pt", async function() {
    await looptest( 97, 0, lineartopcmu, pcmutolinear )
  } )

  it( "trancode pcmu <==> ilbc with dynamic pt", async function() {
    await looptest( 97, 0, lineartopcmu, pcmutolinear, 123 )
  } )

  it( "trancode pcmu <==> g722", async function() {
    await looptest( 9, 0, lineartopcmu, pcmutolinear )
  } )

  it( "trancode pcmu <==> pcma", async function() {
    await looptest( 8, 0, lineartopcmu, pcmutolinear )
  } )

  it( "trancode pcma <==> ilbc", async function() {
    await looptest( 97, 8, lineartopcma, pcmatolinear )
  } )

  it( "trancode pcma <==> g722", async function() {
    await looptest( 9, 8, lineartopcma, pcmatolinear )
  } )

  it( "trancode pcma <==> pcmu", async function() {
    await looptest( 0, 8, lineartopcma, pcmatolinear )
  } )

  it( "trancode pcma <==> pcma", async function() {
    await looptest( 8, 8, lineartopcma, pcmatolinear )
  } )

  it( "trancode pcmu <==> pcmu", async function() {
    await looptest( 0, 0, lineartopcmu, pcmutolinear )
  } )

  it( "simulate an xfer with multiple mix then test new path pcma <==> g722", async function() {

    this.timeout( 8000 )
    this.slow( 7000 )

    /* make sure we have some tone to play */
    projectrtp.tone.generate( "300*0.5:2000", "/tmp/tone.wav" )
    projectrtp.tone.generate( "800*0.5:2000", "/tmp/hightone.wav" )

    /**
     * a and b is the 2 phone legs, c is the transfered channel
     */
    const acodec = 8
    const bcodec = 0
    const ccodec = 9

    /*
    a = 8, b = 0, c = 0 missing tones on a
    a = 8, b = 9, c = 0 all works
    a = 8, b = 0, c = 9, missing tones and missing c leg back on a
    */

    const a = dgram.createSocket( "udp4" )
    const b = dgram.createSocket( "udp4" )
    const c = dgram.createSocket( "udp4" )

    a.bind()
    await new Promise( resolve => a.on( "listening", resolve ) )
    b.bind()
    await new Promise( resolve => b.on( "listening", resolve ) )
    c.bind()
    await new Promise( resolve => c.on( "listening", resolve ) )

    /* echo straight back */
    c.on( "message", function( msg ) {
      c.send( msg, cchannel.local.port, "localhost" )
    } )

    b.on( "message", function( msg ) {
      b.send( msg, bchannel.local.port, "localhost" )
    } )

    let received = Buffer.alloc( 0 )
    let ondonereceiving, recvcount = 0
    const receiveuntil = new Promise( resolve => ondonereceiving = resolve )
    a.on( "message", function( msg ) {
      const pk = parsepk( msg )
      received = Buffer.concat( [ received, pk.payload ] )
      if( 100 < recvcount++ ) ondonereceiving()
    } )

    let done
    const finished = new Promise( ( resolve ) => { done = resolve } )

    let unmixresolve
    const unmixdone = new Promise( resolve => unmixresolve = resolve )

    /* This channel reflects the outbound channel */
    const achannel = await projectrtp.openchannel( { "id": "4" }, function( d ) {
      if( "close" === d.action ) {
        a.close()
        b.close()
        c.close()
        cchannel.close()
      }

      if( "mix" === d.action && "finished" === d.event ) unmixresolve()
    } )

    await new Promise( resolve => setTimeout( resolve, 800 ) )
    achannel.remote( { "address": "localhost", "port": a.address().port, "codec": acodec } )

    /* This channel reflects the originator */
    const bchannel = await projectrtp.openchannel( { "id": "5", "remote": { "address": "localhost", "port": b.address().port, "codec": bcodec } }, function( /*d*/ ) {
    } )

    achannel.mix( bchannel )
    await new Promise( resolve => setTimeout( resolve, 500 ) ) // we really should wait for the mix start events

    /* for some reason in our lib this gets sent again */
    achannel.remote( { "address": "localhost", "port": a.address().port, "codec": acodec } )
    bchannel.remote( { "address": "localhost", "port": b.address().port, "codec": bcodec } )

    achannel.mix( bchannel )
    bchannel.record( { file: "/tmp/test.wav", numchannels: 2, mp3: true } )

    const y = gensignal( 100 )
    const encoded = lineartopcma( Array.from( y ) )

    for( let i = 0 ; 120 > i; i ++ ) {
      sendpk( i, achannel.local.port, a, acodec, 44, encoded, 6300 )
    }

    await new Promise( resolve => setTimeout( resolve, 1200 ) )

    /* Now our blind xfer happens */
    achannel.unmix()
    bchannel.unmix()
    await unmixdone
    
    /* moh followed by ringing tone */
    achannel.play( { "loop": true, "files": [ { "wav": "/tmp/tone.wav" } ] } )
    await new Promise( resolve => setTimeout( resolve, 200 ) )
    achannel.play( { "loop": true, "files": [ { "wav": "/tmp/hightone.wav" } ] } )

    await new Promise( resolve => setTimeout( resolve, 200 ) )

    /* now open our new leg */
    const cchannel = await projectrtp.openchannel( { "id": "6" }, function( d ) {
      if( "close" === d.action ) done()
    } )

    bchannel.close()
    cchannel.remote( { "address": "localhost", "port": c.address().port, "codec": ccodec } )
    await new Promise( resolve => setTimeout( resolve, 40 ) )
    cchannel.mix( achannel )

    /* now we have one way audio in real life */
    await receiveuntil
    const y2 = pcmatolinear( Array.from( received ) )

    await new Promise( resolve => setTimeout( resolve, 1000 ) )
    achannel.close()
    await finished

    npl.plot( [ {
      y: Array.from( y ),
      type: "scatter"
    } ] )

    npl.plot( [ {
      y: Array.from( y2 ),
      type: "scatter"
    } ] )

    /* TODO this currently doesn't test the c leg as this is teh same frequency as the a leg*/
    const amps = ampbyfrequency( y2 )
    expect( has( amps, 100, 25000000 ) ).to.be.true
    expect( has( amps, 300, 25000000 ) ).to.be.true
    expect( has( amps, 800, 25000000 ) ).to.be.true
    expect( has( amps, 500, 25000000 ) ).to.be.false

    await fs.promises.unlink( "/tmp/ukringing.wav" ).catch( () => {} )
  } )

  it( "replay captured g722 from poly", async () => {

    const g722endpoint = dgram.createSocket( "udp4" )
    g722endpoint.on( "message", function() {} )

    const pcmuendpoint = dgram.createSocket( "udp4" )
    let receivedpcmu = []
    pcmuendpoint.on( "message", function( msg ) {
      pcmuendpoint.send( msg, pcmuchannel.local.port, "localhost" )

      receivedpcmu = [ ...receivedpcmu,  ...Array.from( pcmutolinear( parsepk( msg ).payload ) ) ]
    } )

    g722endpoint.bind()
    await new Promise( resolve => g722endpoint.on( "listening", resolve ) )
    pcmuendpoint.bind()
    await new Promise( resolve => pcmuendpoint.on( "listening", resolve ) )

    const allstats = {}

    const g722channel = await projectrtp.openchannel( { "id": "4", "remote": { "address": "localhost", "port": g722endpoint.address().port, "codec": 9 } }, function( d ) {
      if( "close" === d.action ) {
        g722endpoint.close()
        pcmuendpoint.close()
        pcmuchannel.close()
        allstats.achannel = { stats: d.stats }
      }
    } )

    let done
    const allclose = new Promise( resolve => done = resolve )
    const pcmuchannel = await projectrtp.openchannel( { "id": "4", "remote": { "address": "localhost", "port": pcmuendpoint.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {
        allstats.bchannel = { stats: d.stats }
        done()
      }
    } )

    const ourpcap = ( await pcap.readpcap( "test/interface/pcaps/440hzinbackgroundg722.pcap" ) ).slice( 0, 50 )

    g722channel.mix( pcmuchannel )

    const offset = 0
    ourpcap.forEach( ( packet ) => {
      if( packet.ipv4 && packet.ipv4.udp && 10018 == packet.ipv4.udp.dstport ) {
        sendpayload( ( 1000 * packet.ts_sec_offset ) - offset, packet.ipv4.udp.data, g722channel.local.port, g722endpoint )
      }
    } )

    await new Promise( resolve => setTimeout( resolve, 1400 ) )
    g722channel.close()
    await allclose

    npl.plot( [ {
      y: Array.from( receivedpcmu ),
      type: "scatter"
    } ] )

    const amps = ampbyfrequency( Int16Array.from( receivedpcmu ) )
    const bin = 225
    expect( 20000 < amps[ bin ] ).to.be.true

    npl.plot( [ {
      y: Array.from( amps ),
      type: "scatter"
    } ] )

  } )

  it( "replay captured g722 no transcode from poly 3 way mix", async () => {

    const g722endpoint = dgram.createSocket( "udp4" )
    g722endpoint.on( "message", function() {} )

    const pcmuendpoint = dgram.createSocket( "udp4" )
    let receivedpcmu = []

    pcmuendpoint.on( "message", function( msg ) {
      pcmuendpoint.send( msg, pcmuchannel.local.port, "localhost" )

      receivedpcmu = [ ...receivedpcmu,  ...Array.from( pcmutolinear( parsepk( msg ).payload ) ) ]
    } )

    g722endpoint.bind()
    await new Promise( resolve => g722endpoint.on( "listening", resolve ) )
    pcmuendpoint.bind()
    await new Promise( resolve => pcmuendpoint.on( "listening", resolve ) )

    const allstats = {}

    const g722channel = await projectrtp.openchannel( { "id": "4", "remote": { "address": "localhost", "port": g722endpoint.address().port, "codec": 9 } }, function( d ) {
      if( "close" === d.action ) {
        g722endpoint.close()
        pcmuendpoint.close()
        pcmuchannel.close()
        secondg722.close()
        allstats.achannel = { stats: d.stats }
      }
    } )

    let done
    const allclose = new Promise( resolve => done = resolve )
    const pcmuchannel = await projectrtp.openchannel( { "id": "4", "remote": { "address": "localhost", "port": pcmuendpoint.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {
        allstats.bchannel = { stats: d.stats }
        done()
      }
    } )

    const secondg722 = await projectrtp.openchannel( { "id": "4", "remote": { "address": "localhost", "port": 9990, "codec": 9 } }, function( d ) {
      if( "close" === d.action ) {
        allstats.bchannel = { stats: d.stats }
        done()
      }
    } )

    const ourpcap = ( await pcap.readpcap( "test/interface/pcaps/440hzinbackgroundg722.pcap" ) ).slice( 0, 50 )

    g722channel.mix( pcmuchannel )
    g722channel.mix( secondg722 )

    const offset = 0
    ourpcap.forEach( ( packet ) => {
      if( packet.ipv4 && packet.ipv4.udp && 10018 == packet.ipv4.udp.dstport ) {
        sendpayload( ( 1000 * packet.ts_sec_offset ) - offset, packet.ipv4.udp.data, g722channel.local.port, g722endpoint )
      }
    } )

    await new Promise( resolve => setTimeout( resolve, 1400 ) )
    g722channel.close()
    await allclose

    npl.plot( [ {
      y: Array.from( receivedpcmu ),
      type: "scatter"
    } ] )

    const amps = ampbyfrequency( Int16Array.from( receivedpcmu ) )

    npl.plot( [ {
      y: Array.from( amps ),
      type: "scatter"
    } ] )

    const bin = 430
    expect( 20000 < amps[ bin ] ).to.be.true

  } )
} )

