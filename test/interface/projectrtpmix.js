


const expect = require( "chai" ).expect
const dgram = require( "dgram" )
const projectrtp = require( "../../index.js" ).projectrtp
const fs = require( "node:fs" ).promises

function sendpk( sn, sendtime, dstport, server, data = undefined, pt = 0, ssrc = 25 ) {

  const pklength = 172

  return setTimeout( () => {

    let payload
    if( undefined !== data ) {
      payload = data
    } else {
      payload = Buffer.alloc( pklength - 12 ).fill( projectrtp.codecx.linear162pcmu( sn ) & 0xff )
    }

    const subheader = Buffer.alloc( 10 )

    const ts = sn * 160

    subheader.writeUInt16BE( ( sn + 100 ) % ( 2**16 ) /* just some offset */ )
    subheader.writeUInt32BE( ts, 2 )
    subheader.writeUInt32BE( ssrc, 6 )

    const header = Buffer.from( [ 0x80, 0x00 ] )
    header.writeUInt8( pt, 1 ) // payload type

    const rtppacket = Buffer.concat( [
      header,
      subheader,
      payload ] )

    server.send( rtppacket, dstport, "localhost" )
  }, sendtime * 20 )
}


describe( "channel mix", function() {

  this.beforeAll( () => {
    projectrtp.tone.generate( "400+450*0.5/0/400+450*0.5/0:400/200/400/2000", "/tmp/ukringing.wav" )
  } )

  this.afterAll( async () => {
    await fs.unlink( "/tmp/ukringing.wav" )
  } )

  it( "basic mix 2 channels", async function() {

    this.timeout( 3000 )
    this.slow( 2000 )

    const endpointa = dgram.createSocket( "udp4" )
    const endpointb = dgram.createSocket( "udp4" )

    let endpointapkcount = 0
    let endpointbpkcount = 0

    endpointa.on( "message", function( msg ) {
      endpointapkcount++
      expect( msg.length ).to.equal( 172 )
      expect( 0x7f & msg [ 1 ] ).to.equal( 0 )
    } )

    endpointb.on( "message", function( msg ) {
      endpointbpkcount++
      expect( msg.length ).to.equal( 172 )
      expect( 0x7f & msg [ 1 ] ).to.equal( 0 )
      endpointb.send( msg, channelb.local.port, "localhost" )
    } )

    endpointa.bind()
    await new Promise( ( r ) => { endpointa.on( "listening", function() { r() } ) } )

    endpointb.bind()
    await new Promise( ( r ) => { endpointb.on( "listening", function() { r() } ) } )

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) channelb.close()
    } )

    const channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointb.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) done()
    } )

    const rsa = await channela.readstream( {"combined": true });

    const receivedchunksa = [];

    rsa.on( 'data', ( chunk ) => {
      receivedchunksa.push( chunk )
    } )

    const rsb = await channelb.readstream();

    const receivedchunksb = [];

    rsb.on( 'data', ( chunk ) => {
      receivedchunksb.push( chunk )
    } )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true

    /* Now, when we send UDP on endpointb it  passes through our mix then arrives at endpointa */
    for( let i = 0;  50 > i; i ++ ) {
      sendpk( i, i, channela.local.port, endpointa )
    }

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 1300 ) } )

    channela.close()
    endpointa.close()
    endpointb.close()

    expect( endpointapkcount ).to.be.within( 30, 51 )
    expect( endpointbpkcount ).to.be.within( 30, 51 )

    expect( receivedchunksa.length ).to.be.greaterThan( 0 )
    expect( receivedchunksa[ 0 ] ).to.be.instanceOf( Buffer )
    expect( receivedchunksa[ 0 ].length ).to.be.greaterThan( 0 )
    expect( receivedchunksb.length ).to.be.greaterThan( 0 )
    expect( receivedchunksb[ 0 ] ).to.be.instanceOf( Buffer )
    expect( receivedchunksb[ 0 ].length ).to.be.greaterThan( 0 )
    await finished

  } )

  it( "basic mix 2 channels with start 2 packets wrong payload type", async function() {

    this.timeout( 3000 )
    this.slow( 2000 )

    const endpointa = dgram.createSocket( "udp4" )
    const endpointb = dgram.createSocket( "udp4" )

    let endpointapkcount = 0
    let endpointbpkcount = 0

    endpointa.on( "message", function( msg ) {
      endpointapkcount++
      expect( msg.length ).to.equal( 172 )
      expect( 0x7f & msg [ 1 ] ).to.equal( 8 )
    } )

    endpointb.on( "message", function( msg ) {
      endpointbpkcount++

      expect( msg.length ).to.equal( 172 )
      expect( 0x7f & msg [ 1 ] ).to.equal( 0 )
      endpointb.send( msg, channelb.local.port, "localhost" )
    } )

    endpointa.bind()
    await new Promise( ( r ) => { endpointa.on( "listening", function() { r() } ) } )

    endpointb.bind()
    await new Promise( ( r ) => { endpointb.on( "listening", function() { r() } ) } )

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const channela = await projectrtp.openchannel( {}, function( d ) {
      if( "close" === d.action ) channelb.close()
    } )

    const channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointb.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) done()
    } )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true

    /* a problem was highlighted that would remote would be called before rtp stream was updated */
    channela.remote( { "address": "localhost", "port": endpointa.address().port, "codec": 8 } )

    sendpk( 0, 0, channela.local.port, endpointa )
    sendpk( 1, 1, channela.local.port, endpointa )

    /* Now, when we send UDP on endpointb it  passes through our mix then arrives at endpointa */
    for( let i = 2;  50 > i; i ++ ) {
      sendpk( i, i, channela.local.port, endpointa, undefined, 8, 27 )
    }

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 1300 ) } )

    channela.close()
    endpointa.close()
    endpointb.close()

    expect( endpointapkcount ).to.be.within( 30, 51 )
    expect( endpointbpkcount ).to.be.within( 30, 51 )

    await finished

  } )


  it( "mix 2 channels then unmix", async function() {

    this.timeout( 3000 )
    this.slow( 2000 )

    const endpointa = dgram.createSocket( "udp4" )
    const endpointb = dgram.createSocket( "udp4" )

    let endpointapkcount = 0
    let endpointbpkcount = 0

    endpointa.on( "message", function( msg ) {
      endpointapkcount++
      expect( msg.length ).to.equal( 172 )
      expect( 0x7f & msg [ 1 ] ).to.equal( 0 )
    } )

    endpointb.on( "message", function( msg ) {
      endpointbpkcount++
      expect( msg.length ).to.equal( 172 )
      expect( 0x7f & msg [ 1 ] ).to.equal( 0 )
      endpointb.send( msg, channelb.local.port, "localhost" )
    } )

    endpointa.bind()
    await new Promise( ( resolve ) => { endpointa.on( "listening", function() { resolve() } ) } )

    endpointb.bind()
    await new Promise( ( resolve ) => { endpointb.on( "listening", function() { resolve() } ) } )

    let promisearesolve, promisebresolve
    const promisea = new Promise( r => promisearesolve = r )
    const promiseb = new Promise( r => promisebresolve = r )
    
    const channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {
        promisearesolve()
      }
    } )

    const channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointb.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {
        promisebresolve()
      }
    } )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true
    /* Now, when we send UDP on endpointb it  passes through our mix then arrives at endpointa */
    for( let i = 0;  50 > i; i ++ ) {
      sendpk( i, i, channela.local.port, endpointa )
    }

    /* 1S for 50 packets to get through, 0.5 seconds to allow all through jitter buffers */
    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 1700 ) } )

    channelb.unmix()
    channelb.close()
    channela.close()
    endpointa.close()
    endpointb.close()

    /* have we cleaned up? */
    await Promise.all( [ promisea, promiseb ] )

    expect( endpointapkcount ).to.be.within( 49, 51 )
    expect( endpointbpkcount ).to.be.within( 49, 51 )

  } )


  it( "mix 2 channels then unmix then mix again", async function() {

    this.timeout( 6000 )
    this.slow( 5000 )

    const endpointa = dgram.createSocket( "udp4" )
    const endpointb = dgram.createSocket( "udp4" )

    const endpointapkcount = [ 0, 0, 0, 0 ]
    const endpointbpkcount = [ 0, 0, 0, 0 ]

    endpointa.on( "message", function( msg ) {
      endpointapkcount[ 0x7f & msg[ 50 ] ]++
      expect( msg.length ).to.equal( 172 )
      expect( 0x7f & msg [ 1 ] ).to.equal( 0 )
    } )

    endpointb.on( "message", function( msg ) {
      endpointbpkcount[ 0x7f & msg[ 50 ] ]++
      expect( msg.length ).to.equal( 172 )
      expect( 0x7f & msg [ 1 ] ).to.equal( 0 )
    } )

    endpointa.bind()
    await new Promise( ( resolve ) => { endpointa.on( "listening", function() { resolve() } ) } )

    endpointb.bind()
    await new Promise( ( resolve ) => { endpointb.on( "listening", function() { resolve() } ) } )

    let promisearesolve, promisebresolve
    const promisea = new Promise( r => promisearesolve = r )
    const promiseb = new Promise( r => promisebresolve = r )
    
    const channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {
        promisearesolve()
      }
    } )

    const channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointb.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {
        promisebresolve()
      }
    } )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true
    /* Now, when we send UDP on endpointb it  passes through our mix then arrives at endpointa */
    let dataa = Buffer.alloc( 172 - 12 ).fill( 0 )
    let datab = Buffer.alloc( 172 - 12 ).fill( 1 )

    for( let i = 0;  50 > i; i ++ ) {
      sendpk( i, i, channela.local.port, endpointa, dataa )
      sendpk( i, i, channelb.local.port, endpointb, datab )
    }

    /* 1S for 50 packets to get through, 0.5 seconds to allow all through jitter buffers */
    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 1700 ) } )

    channela.direction( { send: true, recv: false } )
    expect( channelb.unmix( channela ) ).to.be.true
    channelb.play( { "loop": true, "files": [
      { "wav": "/tmp/ukringing.wav" } ] } )

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 200 ) } )

    channela.direction( { send: true, recv: true } )
    expect( channelb.mix( channela ) ).to.be.true
    

    dataa = Buffer.alloc( 172 - 12 ).fill( 2 )
    datab = Buffer.alloc( 172 - 12 ).fill( 3 )
    for( let i = 0;  50 > i; i ++ ) {
      sendpk( i, i, channela.local.port, endpointa, dataa )
      sendpk( i, i, channelb.local.port, endpointb, datab )
    }

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 1700 ) } )

    channelb.close()
    channela.close()
    endpointa.close()
    endpointb.close()

    /* have we cleaned up? */
    await Promise.all( [ promisea, promiseb ] )

    expect( endpointapkcount[ 1 ] ).to.be.within( 49, 51 )
    expect( endpointapkcount[ 3 ] ).to.be.within( 49, 51 )
    expect( endpointbpkcount[ 0 ] ).to.be.within( 49, 51 )
    expect( endpointbpkcount[ 2 ] ).to.be.within( 49, 51 )

  } )

  it( "mix 2 channels then close b", async function() {

    this.timeout( 3000 )
    this.slow( 2000 )

    const endpointa = dgram.createSocket( "udp4" )
    const endpointb = dgram.createSocket( "udp4" )

    let endpointapkcount = 0
    let endpointbpkcount = 0

    endpointa.on( "message", function( msg ) {
      endpointapkcount++
      expect( msg.length ).to.equal( 172 )
      expect( 0x7f & msg [ 1 ] ).to.equal( 0 )
    } )

    endpointb.on( "message", function( msg ) {
      endpointbpkcount++
      expect( msg.length ).to.equal( 172 )
      expect( 0x7f & msg [ 1 ] ).to.equal( 0 )
      endpointb.send( msg, channelb.local.port, "localhost" )
    } )

    endpointa.bind()
    await new Promise( ( resolve ) => { endpointa.on( "listening", function() { resolve() } ) } )

    endpointb.bind()
    await new Promise( ( resolve ) => { endpointb.on( "listening", function() { resolve() } ) } )

    let promisearesolve, promisebresolve
    const promisea = new Promise( r => promisearesolve = r )
    const promiseb = new Promise( r => promisebresolve = r )

    const channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {
        promisearesolve()
      }
    } )

    const channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointb.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {
        promisebresolve()
      }
    } )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true

    /* Now, when we send UDP on endpointb it  passes through our mix then arrives at endpointa */
    for( let i = 0;  50 > i; i ++ ) {
      sendpk( i, i, channela.local.port, endpointa )
    }

    /* 1S for 50 packets @ 20mS 0.5 seconds to allow through jitter buffers */
    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 1700 ) } )

    channelb.close()
    channela.close()
    endpointa.close()
    endpointb.close()

    /* have we closed? */
    await Promise.all( [ promisea, promiseb ] )

    expect( endpointapkcount ).to.be.within( 49, 51 )
    expect( endpointbpkcount ).to.be.within( 49, 51 )

  } )

  it( "mix 2 channels - pcmu <-> pcma", async function() {

    this.timeout( 3000 )
    this.slow( 2000 )

    const endpointa = dgram.createSocket( "udp4" )
    const endpointb = dgram.createSocket( "udp4" )

    let endpointapkcount = 0
    let endpointbpkcount = 0

    endpointa.on( "message", function( msg ) {
      endpointapkcount++
      expect( msg.length ).to.equal( 172 )
      expect( 0x7f & msg [ 1 ] ).to.equal( 0 )
    } )

    endpointb.on( "message", function( msg ) {
      endpointbpkcount++
      expect( msg.length ).to.equal( 172 )
      expect( 0x7f & msg [ 1 ] ).to.equal( 8 )
      endpointb.send( msg, channelb.local.port, "localhost" )
    } )

    endpointa.bind()
    await new Promise( ( resolve ) => { endpointa.on( "listening", function() { resolve() } ) } )

    endpointb.bind()
    await new Promise( ( resolve ) => { endpointb.on( "listening", function() { resolve() } ) } )

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) channelb.close()
    } )

    const channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointb.address().port, "codec": 8 } }, function( d ) {
      if( "close" === d.action ) done()
    } )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true

    /* Now, when we send UDP on endpointb it  passes through our mix then arrives at endpointa */
    for( let i = 0;  50 > i; i ++ ) {
      sendpk( i, i, channela.local.port, endpointa )
    }

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 1500 ) } )

    channela.close()
    endpointa.close()
    endpointb.close()

    expect( endpointapkcount ).to.be.within( 30, 51 )
    expect( endpointbpkcount ).to.be.within( 30, 51 )

    await finished

  } )

  it( "mix 2 channels - pcmu <-> ilbc", async function() {

    this.timeout( 3000 )
    this.slow( 2500 )

    const bssrc = 1

    const endpointa = dgram.createSocket( "udp4" )
    const endpointb = dgram.createSocket( "udp4" )

    let endpointapkcount = 0
    let endpointbpkcount = 0

    endpointa.on( "message", function( msg ) {
      endpointapkcount++

      expect( msg.length ).to.equal( 172 )
      expect( 0x7f & msg [ 1 ] ).to.equal( 0 )
    } )

    endpointb.on( "message", function( msg ) {
      endpointbpkcount++

      expect( msg.length ).to.equal( 50 )
      expect( 0x7f & msg [ 1 ] ).to.equal( 97 )

      msg.writeUInt32BE( bssrc, 8 )
      endpointb.send( msg, channelb.local.port, "localhost" )
    } )

    endpointa.bind()
    await new Promise( ( resolve ) => { endpointa.on( "listening", function() { resolve() } ) } )

    endpointb.bind()
    await new Promise( ( resolve ) => { endpointb.on( "listening", function() { resolve() } ) } )

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) channelb.close()
    } )

    const channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointb.address().port, "codec": 97 } }, function( d ) {
      if( "close" === d.action ) done()
    } )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true

    /* Now, when we send UDP on endpointb it  passes through our mix then arrives at endpointa */
    for( let i = 0;  50 > i; i ++ ) {
      sendpk( i, i, channela.local.port, endpointa )
    }

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 1400 ) } )

    channela.close()
    endpointa.close()
    endpointb.close()

    expect( endpointapkcount ).to.be.above( 30 )
    expect( endpointbpkcount ).to.be.above( 30 )

    await finished
  } )

  it( "mix 2 channels - pcmu <-> g722", async function() {

    this.timeout( 3000 )
    this.slow( 2000 )

    const endpointa = dgram.createSocket( "udp4" )
    const endpointb = dgram.createSocket( "udp4" )

    let endpointapkcount = 0
    let endpointbpkcount = 0

    endpointa.on( "message", function( msg ) {
      endpointapkcount++
      expect( 0x7f & msg [ 1 ] ).to.equal( 0 )
    } )

    endpointb.on( "message", function( msg ) {

      endpointbpkcount++
      expect( 0x7f & msg [ 1 ] ).to.equal( 9 )
      endpointb.send( msg, channelb.local.port, "localhost" )
    } )

    endpointa.bind()
    await new Promise( ( resolve ) => { endpointa.on( "listening", function() { resolve() } ) } )

    endpointb.bind()
    await new Promise( ( resolve ) => { endpointb.on( "listening", function() { resolve() } ) } )

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) channelb.close()
    } )

    const channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointb.address().port, "codec": 9 } }, function( d ) {
      if( "close" === d.action ) done()
    } )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true

    /* Now, when we send UDP on endpointb it  passes through our mix then arrives at endpointa */
    for( let i = 0;  50 > i; i ++ ) {
      sendpk( i, i, channela.local.port, endpointa )
    }

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 1500 ) } )

    channela.close()
    endpointa.close()
    endpointb.close()

    expect( endpointapkcount ).to.be.above( 30 )
    expect( endpointbpkcount ).to.be.above( 30 )

    await finished

  } )

  it( "mix 2 channels - pcmu <-> g722 with recording", async function() {

    this.timeout( 3000 )
    this.slow( 2000 )

    const endpointa = dgram.createSocket( "udp4" )
    const endpointb = dgram.createSocket( "udp4" )

    let endpointapkcount = 0
    let endpointbpkcount = 0

    endpointa.on( "message", function( msg ) {
      endpointapkcount++
      expect( 0x7f & msg [ 1 ] ).to.equal( 0 )
    } )

    endpointb.on( "message", function( msg ) {

      endpointbpkcount++
      expect( 0x7f & msg [ 1 ] ).to.equal( 9 )
      endpointb.send( msg, channelb.local.port, "localhost" )
    } )

    endpointa.bind()
    await new Promise( ( resolve ) => { endpointa.on( "listening", function() { resolve() } ) } )

    endpointb.bind()
    await new Promise( ( resolve ) => { endpointb.on( "listening", function() { resolve() } ) } )

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) channelb.close()
    } )

    const channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointb.address().port, "codec": 9 } }, function( d ) {
      if( "close" === d.action ) done()
    } )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true

    channela.record( { "file": "/tmp/g722mix2recording.wav" } )

    /* Now, when we send UDP on endpointb it  passes through our mix then arrives at endpointa */
    for( let i = 0;  50 > i; i ++ ) {
      sendpk( i, i, channela.local.port, endpointa )
    }

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 1500 ) } )

    channela.close()
    endpointa.close()
    endpointb.close()

    expect( endpointapkcount ).to.be.above( 30 )
    expect( endpointbpkcount ).to.be.above( 30 )

    await finished

  } )


  it( "playback prompt then mix 2 channels - pcmu <-> g722 with recording", async function() {

    this.timeout( 3000 )
    this.slow( 2000 )

    const endpointa = dgram.createSocket( "udp4" )
    const endpointb = dgram.createSocket( "udp4" )

    let endpointapkcount = 0
    let endpointbpkcount = 0

    endpointa.on( "message", function( msg ) {
      endpointapkcount++
      expect( 0x7f & msg [ 1 ] ).to.equal( 0 )
    } )

    endpointb.on( "message", function( msg ) {

      endpointbpkcount++
      expect( 0x7f & msg [ 1 ] ).to.equal( 9 )
      endpointb.send( msg, channelb.local.port, "localhost" )
    } )

    endpointa.bind()
    await new Promise( ( resolve ) => { endpointa.on( "listening", function() { resolve() } ) } )

    endpointb.bind()
    await new Promise( ( resolve ) => { endpointb.on( "listening", function() { resolve() } ) } )

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) channelb.close()
    } )

    channela.play( { "loop": true, "files": [
      { "wav": "/tmp/ukringing.wav" } ] } )

    await new Promise( resolve => setTimeout( resolve, 500 ) )

    const channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointb.address().port, "codec": 9 } }, function( d ) {
      if( "close" === d.action ) done()
    } )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true

    channela.record( { "file": "/tmp/g722mix2recording.wav" } )

    /* Now, when we send UDP on endpointb it  passes through our mix then arrives at endpointa */
    for( let i = 0;  50 > i; i ++ ) {
      sendpk( i, i, channela.local.port, endpointa )
    }

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 1500 ) } )

    channela.close()
    endpointa.close()
    endpointb.close()

    expect( endpointapkcount ).to.be.above( 30 )
    expect( endpointbpkcount ).to.be.above( 30 )

    await finished

  } )

  it( "mix 3 channels - 1 writer 3 readers", async function() {

    this.timeout( 3000 )
    this.slow( 2000 )

    const endpointa = dgram.createSocket( "udp4" )
    const endpointb = dgram.createSocket( "udp4" )
    const endpointc = dgram.createSocket( "udp4" )

    let endpointapkcount = 0
    let endpointbpkcount = 0

    endpointa.on( "message", function( msg ) {
      endpointapkcount++
      expect( projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ).to.be.oneOf([0 , 8 ] )
    } )

    endpointb.on( "message", function( msg ) {
      endpointbpkcount++
      expect( projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ).to.be.oneOf([0 , 8 ] )
    } )

    endpointc.on( "message", function( msg ) {
      expect( projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ).to.be.oneOf([0 , 8 ] )
    } )

    endpointa.bind()
    await new Promise( ( resolve ) => { endpointa.on( "listening", function() { resolve() } ) } )

    endpointb.bind()
    await new Promise( ( resolve ) => { endpointb.on( "listening", function() { resolve() } ) } )

    endpointc.bind()
    await new Promise( ( resolve ) => { endpointc.on( "listening", function() { resolve() } ) } )

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) channelb.close()
    } )

    const channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointb.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) channelc.close()
    } )

    const channelc = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointc.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) done()
    } )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true
    expect( channela.mix( channelc ) ).to.be.true

    /* Now, when we send UDP on endpointb it  passes through our mix then arrives at endpointa */
    for( let i = 0;  50 > i; i ++ ) {
      sendpk( i, i, channelc.local.port, endpointc, Buffer.alloc( 160 ).fill( projectrtp.codecx.linear162pcmu( 8 ) ) )
    }

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 1200 ) } )

    channela.close()
    endpointa.close()
    endpointb.close()
    endpointc.close()

    /* This value is based on timeing so may vary very slightly */
    expect( endpointapkcount ).to.be.within( 59, 61 )
    expect( endpointbpkcount ).to.be.within( 59, 61 )

    await finished

  } )

  it( "mix 3 channels - 1 writer 1 readers (2 silenced)", async function() {

    this.timeout( 3000 )
    this.slow( 2000 )

    const endpointa = dgram.createSocket( "udp4" )
    const endpointb = dgram.createSocket( "udp4" )
    const endpointc = dgram.createSocket( "udp4" )

    let endpointapkcount = 0
    let endpointbpkcount = 0
    let endpointcpkcount = 0

    endpointa.on( "message", function( msg ) {
      endpointapkcount++
      expect( projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ).to.be.oneOf([0 , 8 ] )
    } )

    endpointb.on( "message", function( msg ) {
      endpointbpkcount++
      expect( projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ).to.be.oneOf([0 , 8 ] )
    } )

    endpointc.on( "message", function( msg ) {
      endpointcpkcount++
      expect( projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ).to.be.oneOf([0 , 8 ] )
    } )

    endpointa.bind()
    await new Promise( ( resolve ) => { endpointa.on( "listening", function() { resolve() } ) } )

    endpointb.bind()
    await new Promise( ( resolve ) => { endpointb.on( "listening", function() { resolve() } ) } )

    endpointc.bind()
    await new Promise( ( resolve ) => { endpointc.on( "listening", function() { resolve() } ) } )

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const channela = await projectrtp.openchannel( { "direction": { "send": false }, "remote": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) channelb.close()
    } )

    const channelb = await projectrtp.openchannel( { "direction": { "send": false }, "remote": { "address": "localhost", "port": endpointb.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) channelc.close()
    } )

    const channelc = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointc.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) done()
    } )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true
    expect( channela.mix( channelc ) ).to.be.true

    /* Now, when we send UDP on endpointb it  passes through our mix then arrives at endpointa */
    for( let i = 0;  50 > i; i ++ ) {
      sendpk( i, i, channelc.local.port, endpointc, Buffer.alloc( 160 ).fill( projectrtp.codecx.linear162pcmu( 8 ) ) )
    }

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 1200 ) } )

    channela.close()
    endpointa.close()
    endpointb.close()
    endpointc.close()

    expect( endpointapkcount ).to.be.equal( 0 )
    expect( endpointbpkcount ).to.be.equal( 0 )
    expect( endpointcpkcount ).to.be.within( 59, 61 )

    await finished

  } )

  it( "mix 3 channels - 1 writer 3 recevers but writer recv=false", async function() {

    this.timeout( 3000 )
    this.slow( 2000 )

    const endpointa = dgram.createSocket( "udp4" )
    const endpointb = dgram.createSocket( "udp4" )
    const endpointc = dgram.createSocket( "udp4" )

    let endpointapkcount = 0
    let endpointbpkcount = 0
    let endpointcpkcount = 0

    endpointa.on( "message", function( msg ) {
      endpointapkcount++
      expect( projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ).to.equal( 0 )
    } )

    endpointb.on( "message", function( msg ) {
      endpointbpkcount++
      expect( projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ).to.equal( 0 )
    } )

    endpointc.on( "message", function( msg ) {
      endpointcpkcount++
      expect( projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ).to.equal( 0 )
    } )

    endpointa.bind()
    await new Promise( ( r ) => { endpointa.on( "listening", function() { r() } ) } )

    endpointb.bind()
    await new Promise( ( r ) => { endpointb.on( "listening", function() { r() } ) } )

    endpointc.bind()
    await new Promise( ( r ) => { endpointc.on( "listening", function() { r() } ) } )

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) channelb.close()
    } )

    const channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointb.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) channelc.close()
    } )

    const channelc = await projectrtp.openchannel( { "direction": { "recv": false }, "remote": { "address": "localhost", "port": endpointc.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) done()
    } )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true
    expect( channela.mix( channelc ) ).to.be.true

    /* Send data - which should be ignored but receviers should recevie silence (in payload) */
    for( let i = 0;  50 > i; i ++ ) {
      sendpk( i, i, channelc.local.port, endpointc )
    }

    await new Promise( ( r ) => { setTimeout( () => r(), 1500 ) } )

    channela.close()
    endpointa.close()
    endpointb.close()
    endpointc.close()

    expect( endpointapkcount ).to.be.within( 70, 80 )
    expect( endpointbpkcount ).to.be.within( 70, 80 )
    expect( endpointcpkcount ).to.be.within( 70, 80 )

    await finished

  } )


  it( "mix 3 channels - 1 writer 3 recevers but writer delayed recv=false", async function() {

    this.timeout( 3000 )
    this.slow( 2000 )

    const endpointa = dgram.createSocket( "udp4" )
    const endpointb = dgram.createSocket( "udp4" )
    const endpointc = dgram.createSocket( "udp4" )


    let endpointapkcountzero = 0
    let endpointbpkcountzero = 0
    let endpointcpkcountzero = 0
    let endpointapkcountnotzero = 0
    let endpointbpkcountnotzero = 0
    let endpointcpkcountnotzero = 0

    endpointa.on( "message", function( msg ) {
      if( 0 == projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ) {
        endpointapkcountzero++
      } else {
        endpointapkcountnotzero++
      }
    } )

    endpointb.on( "message", function( msg ) {
      if( 0 == projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ) {
        endpointbpkcountzero++
      } else {
        endpointbpkcountnotzero++
      }
    } )

    endpointc.on( "message", function( msg ) {
      if( 0 == projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ) {
        endpointcpkcountzero++
      } else {
        endpointcpkcountnotzero++
      }
    } )

    endpointa.bind()
    await new Promise( ( r ) => { endpointa.on( "listening", function() { r() } ) } )

    endpointb.bind()
    await new Promise( ( r ) => { endpointb.on( "listening", function() { r() } ) } )

    endpointc.bind()
    await new Promise( ( r ) => { endpointc.on( "listening", function() { r() } ) } )

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) channelb.close()
    } )

    const channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointb.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) channelc.close()
    } )

    const channelc = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointc.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) done()
    } )

    setTimeout( () => expect( channelc.direction( { "recv": false } ) ).to.be.true , 400 )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true
    expect( channela.mix( channelc ) ).to.be.true

    /* Send data - which should be ignored but receviers should recevie silence (in payload) */
    for( let i = 0;  50 > i; i ++ ) {
      sendpk( i, i, channelc.local.port, endpointc, Buffer.alloc( 160 ).fill( projectrtp.codecx.linear162pcmu( 8 ) ) )
    }

    await new Promise( ( r ) => { setTimeout( () => r(), 1500 ) } )

    channela.close()
    endpointa.close()
    endpointb.close()
    endpointc.close()

    expect( endpointapkcountzero ).to.be.within( 55, 75 )
    expect( endpointbpkcountzero ).to.be.within( 55, 75 )
    expect( endpointcpkcountzero ).to.be.within( 55, 75 )
    expect( endpointapkcountnotzero ).to.be.within( 4, 18 )
    expect( endpointbpkcountnotzero ).to.be.within( 4, 18 )
    expect( endpointcpkcountnotzero ).to.be.below( 2 )

    await finished

  } )

  it( "mix 3 channels - 1 writer 1 readers (2 silenced but delayed)", async function() {

    this.timeout( 3000 )
    this.slow( 2000 )

    const endpointa = dgram.createSocket( "udp4" )
    const endpointb = dgram.createSocket( "udp4" )
    const endpointc = dgram.createSocket( "udp4" )

    let endpointapkcount = 0
    let endpointbpkcount = 0
    let endpointcpkcount = 0

    endpointa.on( "message", function( msg ) {
      endpointapkcount++
      expect( projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ).to.be.oneOf([0 , 8 ] )
    } )

    endpointb.on( "message", function( msg ) {
      endpointbpkcount++
      expect( projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ).to.be.oneOf([0 , 8 ] )
    } )

    endpointc.on( "message", function( msg ) {
      endpointcpkcount++
      expect( projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ).to.be.oneOf([0 , 8 ] )
    } )

    endpointa.bind()
    await new Promise( ( resolve ) => { endpointa.on( "listening", function() { resolve() } ) } )

    endpointb.bind()
    await new Promise( ( resolve ) => { endpointb.on( "listening", function() { resolve() } ) } )

    endpointc.bind()
    await new Promise( ( resolve ) => { endpointc.on( "listening", function() { resolve() } ) } )

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) channelb.close()
    } )

    const channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointb.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) channelc.close()
    } )

    const channelc = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointc.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) done()
    } )

    setTimeout( () => expect( channela.direction( { "send": false } ) ).to.be.true , 200 )
    setTimeout( () => expect( channelb.direction( { "send": false } ) ).to.be.true , 200 )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true
    expect( channela.mix( channelc ) ).to.be.true

    /* Now, when we send UDP on endpointb it  passes through our mix then arrives at endpointa */
    for( let i = 0;  50 > i; i ++ ) {
      sendpk( i, i, channelc.local.port, endpointc, Buffer.alloc( 160 ).fill( projectrtp.codecx.linear162pcmu( 8 ) ) )
    }

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 1500 ) } )

    channela.close()
    endpointa.close()
    endpointb.close()
    endpointc.close()

    expect( endpointapkcount ).to.be.at.most( 15 )
    expect( endpointbpkcount ).to.be.at.most( 15 )
    expect( endpointcpkcount ).to.be.within( 59, 75 )

    await finished

  } )
} )
