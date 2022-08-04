
const expect = require( "chai" ).expect
const projectrtp = require( "../../index.js" ).projectrtp
const dgram = require( "dgram" )

const pcap = require( "./pcap.js" )

/*
i.e. the RTP payload
str = "80 e5 03 b5 00 02 44 a0 1e e3 61 fb 03 0a 00 a0 4c d1"
returns
Buffer.from( [ 0x80, ... ] )
*/
function fromstr( str ) {
  let retval = []
  str.split( " " ).forEach( v => retval.push ( parseInt( v, 16 ) ) )
  return Buffer.from( retval )
}

function sendpayload( sendtime, pk, dstport, server ) {
  return setTimeout( () => {
    server.send( pk, dstport, "localhost" )
  }, sendtime )
}

/* helper functions */
function sendpk( sn, sendtime, dstport, server, pt = 0, ts, ssrc ) {

  if( !ssrc ) ssrc = 25
  let pklength = 172

  return setTimeout( () => {

    let payload = Buffer.alloc( pklength - 12 ).fill( projectrtp.codecx.linear162pcmu( sn ) & 0xff )
    let subheader = Buffer.alloc( 10 )

    if( !ts ) ts = sn * 160

    subheader.writeUInt8( pt, 1 ) // payload type
    subheader.writeUInt16BE( ( sn ) % ( 2**16 ) )
    subheader.writeUInt32BE( ts, 2 )
    subheader.writeUInt32BE( ssrc, 6 )

    let rtppacket = Buffer.concat( [
      Buffer.from( [ 0x80, 0x00 ] ),
      subheader,
      payload ] )

    server.send( rtppacket, dstport, "localhost" )
  }, sendtime )
}

/*

*/
function senddtmf( sn, ts, sendtime, dstport, server, endofevent, ev ) {

  return setTimeout( () => {
    let ssrc = 25
    let pklength = 58

    let header = Buffer.alloc( pklength )

    let pt = 101

    header.writeUInt8( 0x80 )
    header.writeUInt8( pt, 1 ) // payload type
    header.writeUInt16BE( ( sn ) % ( 2**16 ), 2 )
    header.writeUInt32BE( ts, 4 )
    header.writeUInt32BE( ssrc, 8 )

    /* DTMF data */
    header.writeUInt8( ev, 12 )

    let eoerv = 10
    if( endofevent ) {
      eoerv = eoerv | 0x80
    }
    header.writeUInt8( eoerv, 13 ) /* End of Event, Reserved, Volume */
    header.writeUInt16BE( 160, 14 ) /* Duration */

    server.send( header, dstport, "localhost" )
  }, sendtime )
}

/* Tests */
describe( "dtmf", function() {

  it( `Send 2833 DTMF and check event`, function( done ) {

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    var receviedpkcount = 0
    server.on( "message", function( msg, rinfo ) {
      receviedpkcount++
    } )

    this.timeout( 3000 )
    this.slow( 2500 )

    server.bind()
    server.on( "listening", async function() {

      let ourport = server.address().port

      let expectedmessagecount = 0
      const expectedmessages = [
        { action: 'telephone-event', event: '4' },
        { action: 'close' }
      ]

      let channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {

        expect( d ).to.deep.include( expectedmessages[ expectedmessagecount ] )
        expectedmessagecount++

        if( "close" === d.action ) {
          server.close()
          done()
        }
      } )

      expect( channel.echo() ).to.be.true

      /* send a packet every 20mS x 50 */
      for( let i = 0;  i < 23; i ++ ) {
        sendpk( i, i*20, channel.local.port, server )
      }

      senddtmf( 23, 22 * 160, 23*20, channel.local.port, server, false, "4" )
      senddtmf( 24, 22 * 160, 24*20, channel.local.port, server, false, "4" )
      senddtmf( 25, 22 * 160, 25*20, channel.local.port, server, true, "4" )

      for( let i = 26;  i < 40; i ++ ) {
        sendpk( i, (i-3)*20, channel.local.port, server )
      }

      setTimeout( () => channel.close(), 1000 )
    } )
  } )


  it( `single channel and request rtp server to send 2833`, function( done ) {

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    var receviedpkcount = 0
    var dtmfpkcount = 0
    server.on( "message", function( msg, rinfo ) {
      receviedpkcount++
      if( 101 == ( 0x7f & msg [ 1 ] ) ) {
        dtmfpkcount++
      } else {
        receviedpkcount++
        expect( msg.length ).to.equal( 172 )
        expect( 0x7f & msg [ 1 ] ).to.equal( 0 )
      }
    } )

    this.timeout( 3000 )
    this.slow( 2500 )

    server.bind()
    server.on( "listening", async function() {

      let ourport = server.address().port

      let expectedmessagecount = 0
      const expectedmessages = [
        { action: 'close' }
      ]

      let channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {

        expect( d ).to.deep.include( expectedmessages[ expectedmessagecount ] )
        expectedmessagecount++

        if( "close" === d.action ) {
          server.close()
          expect( dtmfpkcount ).to.equal( 2*3 )
          done()
        }
      } )

      expect( channel.echo() ).to.be.true

      /* send a packet every 20mS x 50 */
      for( let i = 0;  i < 50; i ++ ) {
        sendpk( i, i*20, channel.local.port, server )
      }

      setTimeout( () => channel.dtmf( "#1" ), 400 )
      setTimeout( () => channel.close(), 1000 )
    } )
  } )


  it( `2 channels mixing and request rtp server to send 2833 to one`, async function() {

    /* create our RTP/UDP endpoint */
    const clienta = dgram.createSocket( "udp4" )
    const clientb = dgram.createSocket( "udp4" )

    var receviedpkcount = 0
    var dtmfpkcount = 0
    clienta.on( "message", function( msg ) {
      receviedpkcount++
      if( 101 == ( 0x7f & msg [ 1 ] ) ) {
        dtmfpkcount++
      } else {
        receviedpkcount++
        expect( msg.length ).to.equal( 172 )
        expect( 0x7f & msg [ 1 ] ).to.equal( 0 )
      }
    } )

    clientb.on( "message", function( msg ) {
      if( 101 == ( 0x7f & msg [ 1 ] ) ) {
        expect( true ).to.equal( false ) //here = bad
        dtmfpkcount++
      }
      clientb.send( msg, channelb.local.port, "localhost" )
    } )

    this.timeout( 3000 )
    this.slow( 2500 )

    clienta.bind()
    await new Promise( ( resolve ) => { clienta.on( "listening", () => resolve()  ) } )
    clientb.bind()
    await new Promise( ( resolve ) => { clientb.on( "listening", () => resolve()  ) } )

    let ouraport = clienta.address().port
    let ourbport = clientb.address().port

    let done
    let finished = new Promise( ( r ) => { done = r } )

    let channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ouraport, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) channelb.close()
    } )

    let channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourbport, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) done()
    } )

    expect( channela.mix( channelb ) ).to.be.true

    /* send a packet every 20mS x 70 */
    for( let i = 0;  i < 50; i ++ ) {
      sendpk( i, i*20, channela.local.port, clienta )
    }

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 400 ) } )
    channela.dtmf( "*9F" )
    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 800 ) } )
    channela.close()

    await finished

    clienta.close()
    clientb.close()

    expect( dtmfpkcount ).to.equal( 3*3 )
  } )

  it( `3 channels mixing and request rtp server to send 2833 to one`, async function() {

    /* create our RTP/UDP endpoint */
    const clienta = dgram.createSocket( "udp4" )
    const clientb = dgram.createSocket( "udp4" )
    const clientc = dgram.createSocket( "udp4" )

    var receviedpkcount = 0
    var dtmfpkcount = 0
    clienta.on( "message", function( msg, rinfo ) {
      receviedpkcount++
      if( 101 == ( 0x7f & msg [ 1 ] ) ) {
        dtmfpkcount++
      } else {
        receviedpkcount++
        expect( msg.length ).to.equal( 172 )
        expect( 0x7f & msg [ 1 ] ).to.equal( 0 )
      }
    } )

    clientb.on( "message", function( msg, rinfo ) {
      if( 101 == ( 0x7f & msg [ 1 ] ) ) {
        expect( true ).to.equal( false ) //here = bad
        dtmfpkcount++
      }
      clientb.send( msg, channelb.local.port, "localhost" )
    } )

    clientc.on( "message", function( msg, rinfo ) {
      if( 101 == ( 0x7f & msg [ 1 ] ) ) {
        expect( true ).to.equal( false ) //here = bad
        dtmfpkcount++
      }
      clientb.send( msg, channelb.local.port, "localhost" )
    } )

    this.timeout( 3000 )
    this.slow( 2500 )

    clienta.bind()
    await new Promise( ( resolve, reject ) => { clienta.on( "listening", () => resolve()  ) } )
    clientb.bind()
    await new Promise( ( resolve, reject ) => { clientb.on( "listening", () => resolve()  ) } )
    clientc.bind()
    await new Promise( ( resolve, reject ) => { clientc.on( "listening", () => resolve()  ) } )

    let ouraport = clienta.address().port
    let ourbport = clientb.address().port
    let ourcport = clientc.address().port

    let done
    let finished = new Promise( ( r ) => { done = r } )

    let channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ouraport, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) channelb.close()
    } )

    let channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourbport, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) channelc.close()
    } )

    let channelc = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourcport, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) done()
    } )

    expect( channela.mix( channelb ) ).to.be.true
    expect( channela.mix( channelc ) ).to.be.true

    /* send a packet every 20mS x 50 */
    for( let i = 0;  i < 50; i ++ ) {
      sendpk( i, i*20, channela.local.port, clienta )
    }

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 100 ) } )
    channela.dtmf( "*9ABD" )
    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 900 ) } )
    channela.close()

    clienta.close()
    clientb.close()
    clientc.close()

    expect( dtmfpkcount ).to.equal( 5*3 )

    await finished
  } )


  it( `Send multiple 2833 DTMF and check event`, function( done ) {

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    var receviedpkcount = 0
    server.on( "message", function( msg, rinfo ) {
      receviedpkcount++
    } )

    this.timeout( 3000 )
    this.slow( 2500 )

    server.bind()
    server.on( "listening", async function() {

      let ourport = server.address().port

      let expectedmessagecount = 0
      const expectedmessages = [
        { action: 'telephone-event', event: '4' },
        { action: 'telephone-event', event: '5' },
        { action: 'close' }
      ]

      let channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {
        expect( d ).to.deep.include( expectedmessages[ expectedmessagecount ] )
        expectedmessagecount++

        if( "close" === d.action ) {
          server.close()
          done()
        }
      } )

      expect( channel.echo() ).to.be.true

      /* send a packet every 20mS x 50 */
      for( let i = 0;  i < 13; i ++ ) {
        sendpk( i, i*20, channel.local.port, server )
      }

      senddtmf( 13, 12 * 160, 13*20, channel.local.port, server, false, "4" )
      senddtmf( 14, 12 * 160, 14*20, channel.local.port, server, false, "4" )
      // Packet loss
      // senddtmf( 15, 12 * 160, 15*20, channel.port, server, true, "4" )

      for( let i = 16;  i < 23; i ++ ) {
        sendpk( i, (i-3)*20, channel.local.port, server )
      }

      senddtmf( 23, 22 * 160, 23*20, channel.local.port, server, false, "5" )
      senddtmf( 24, 22 * 160, 24*20, channel.local.port, server, false, "5" )
      senddtmf( 25, 22 * 160, 25*20, channel.local.port, server, true, "5" )

      for( let i = 26;  i < 33; i ++ ) {
        sendpk( i, (i-6)*20, channel.local.port, server )
      }

      setTimeout( () => channel.close(), 1000 )
    } )
  } )

  it( `Lose end packet`, async function() {

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    const receivedmessages = []

    var receviedpkcount = 0
    server.on( "message", function( msg, rinfo ) {
      receviedpkcount++
    } )

    let done
    let finished = new Promise( r => done = r )

    this.timeout( 3000 )
    this.slow( 2500 )

    server.bind()
    server.on( "listening", async function() {

      let ourport = server.address().port
      let channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {
        receivedmessages.push( d )

        if( "close" === d.action ) {
          server.close()
          done()
        }
      } )

      expect( channel.echo() ).to.be.true

      /* send a packet every 20mS x 50 */
      for( let i = 0;  i < 13; i ++ ) {
        sendpk( i, i*20, channel.local.port, server )
      }

      senddtmf( 13, 12 * 160, 13*20, channel.local.port, server, false, "4" )
      senddtmf( 14, 12 * 160, 14*20, channel.local.port, server, false, "4" )
      // Packet loss
      // senddtmf( 15, 12 * 160, 15*20, channel.port, server, true, "4" )

      for( let i = 17;  i < 17+75; i ++ ) {
        sendpk( i, (i-3)*20, channel.local.port, server )
      }

      setTimeout( () => channel.close(), 2600 )

    } )

    await finished

    const expectedmessages = [
      { action: 'telephone-event', event: '4' },
      { action: 'close' }
    ]

    expect( receviedpkcount ).to.be.above( 83 )
    expect( receivedmessages.length ).to.equal( 2 )
    expect( receivedmessages[ 0 ] ).to.deep.include( expectedmessages[ 0 ] )
    expect( receivedmessages[ 1 ] ).to.deep.include( expectedmessages[ 1 ] )
  } )


  it( `mix 2 channels - pcmu <-> pcma and send DTMF`, async function() {

    /*
      When mixing 2 channels, we expect the second leg to receive the 2833 packets
      and our server to emit events indicating the DTMF on the first channel.
    */
    this.timeout( 3000 )
    this.slow( 2000 )

    const endpointa = dgram.createSocket( "udp4" )
    const endpointb = dgram.createSocket( "udp4" )

    const receivedmessages = []

    let endpointapkcount = 0
    let endpointbpkcount = 0
    let dtmfpkcount = 0

    endpointa.on( "message", function( msg, rinfo ) {
      endpointapkcount++
      expect( msg.length ).to.equal( 172 )
      expect( 0x7f & msg [ 1 ] ).to.equal( 0 )
    } )

    endpointb.on( "message", function( msg, rinfo ) {
      endpointbpkcount++
      if( 101 == ( 0x7f & msg [ 1 ] ) ) {
        dtmfpkcount++
      } else {
        expect( msg.length ).to.equal( 172 )
        expect( 0x7f & msg [ 1 ] ).to.equal( 8 )
        endpointb.send( msg, channelb.local.port, "localhost" )
      }
    } )

    endpointa.bind()
    await new Promise( ( resolve ) => { endpointa.on( "listening", function() { resolve() } ) } )

    endpointb.bind()
    await new Promise( ( resolve ) => { endpointb.on( "listening", function() { resolve() } ) } )

    let done
    let finished = new Promise( ( r ) => { done = r } )

    let channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      receivedmessages.push( d )

      if( "close" === d.action ) {
        channelb.close()
      }
    } )

    let channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointb.address().port, "codec": 8 } }, function( d ) {
      if( "close" === d.action ) done()
    } )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true

    /* send a packet every 20mS x 50 */
    for( let i = 0;  i < 13; i ++ ) {
      sendpk( i, i*20, channela.local.port, endpointa )
    }

    senddtmf( 13, 12 * 160, 13*20, channela.local.port, endpointa, false, "4" )
    senddtmf( 14, 12 * 160, 14*20, channela.local.port, endpointa, false, "4" )
    senddtmf( 15, 12 * 160, 15*20, channela.local.port, endpointa, true, "4" )

    for( let i = 16;  i < 23; i ++ ) {
      sendpk( i, (i-3)*20, channela.local.port, endpointa )
    }

    senddtmf( 23, 22 * 160, 23*20, channela.local.port, endpointa, false, "5" )
    senddtmf( 24, 22 * 160, 24*20, channela.local.port, endpointa, false, "5" )
    senddtmf( 25, 22 * 160, 25*20, channela.local.port, endpointa, true, "5" )

    for( let i = 26;  i < 50; i ++ ) {
      sendpk( i, (i-6)*20, channela.local.port, endpointa )
    }

    await new Promise( ( r ) => { setTimeout( () => r(), 1500 ) } )

    channela.close()
    endpointa.close()
    endpointb.close()

    await finished

    expect( endpointapkcount ).to.be.within( 30, 51 )
    expect( endpointbpkcount ).to.be.within( 30, 51 )
    expect( dtmfpkcount ).to.equal( 6 )

    const expectedmessages = [
      { action: 'mix', event: 'start' },
      { action: 'telephone-event', event: '4' },
      { action: 'telephone-event', event: '5' },
      { action: 'mix', event: 'finished' },
      { action: 'close' }
    ]

    expect( receivedmessages.length ).to.equal( 5 )
    expect( receivedmessages[ 0 ] ).to.deep.include( expectedmessages[ 0 ] )
    expect( receivedmessages[ 1 ] ).to.deep.include( expectedmessages[ 1 ] )
    expect( receivedmessages[ 2 ] ).to.deep.include( expectedmessages[ 2 ] )
    expect( receivedmessages[ 3 ] ).to.deep.include( expectedmessages[ 3 ] )
    expect( receivedmessages[ 4 ] ).to.deep.include( expectedmessages[ 4 ] )

  } )


  it( `mix 3 channels - pcmu <-> pcma and ilbc and send DTMF`, async function() {

    this.timeout( 3000 )
    this.slow( 2000 )

    const endpointa = dgram.createSocket( "udp4" )
    const endpointb = dgram.createSocket( "udp4" )
    const endpointc = dgram.createSocket( "udp4" )

    let endpointapkcount = 0
    let endpointbpkcount = 0
    let endpointcpkcount = 0

    let dtmfapkcount = 0
    let dtmfbpkcount = 0
    let dtmfcpkcount = 0

    endpointa.on( "message", function( msg, rinfo ) {
      endpointapkcount++
      if( 101 == ( 0x7f & msg [ 1 ] ) ) {
        dtmfapkcount++
      } else {
        expect( msg.length ).to.equal( 172 )
        expect( 0x7f & msg [ 1 ] ).to.equal( 0 )
        endpointb.send( msg, channelb.local.port, "localhost" )
      }
    } )

    endpointb.on( "message", function( msg, rinfo ) {
      endpointbpkcount++
      if( 101 == ( 0x7f & msg [ 1 ] ) ) {
        dtmfbpkcount++
      } else {
        expect( msg.length ).to.equal( 172 )
        expect( 0x7f & msg [ 1 ] ).to.equal( 8 )
        endpointb.send( msg, channelb.local.port, "localhost" )
      }
    } )

    endpointc.on( "message", function( msg, rinfo ) {
      endpointcpkcount++
      if( 101 == ( 0x7f & msg [ 1 ] ) ) {
        dtmfcpkcount++
      } else {
        expect( msg.length ).to.equal( 50 )
        expect( 0x7f & msg [ 1 ] ).to.equal( 97 )
        endpointb.send( msg, channelb.local.port, "localhost" )
      }
    } )

    endpointa.bind()
    await new Promise( ( r ) => { endpointa.on( "listening", function() { r() } ) } )

    endpointb.bind()
    await new Promise( ( r ) => { endpointb.on( "listening", function() { r() } ) } )

    endpointc.bind()
    await new Promise( ( r ) => { endpointc.on( "listening", function() { r() } ) } )

    let expectedmessagecount = 0
    const expectedmessages = [
      { action: 'telephone-event', event: '4' },
      { action: 'telephone-event', event: '5' },
      { action: 'close' }
    ]

    let done
    let finished = new Promise( ( r ) => { done = r } )

    let channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      expect( d ).to.deep.include( expectedmessages[ expectedmessagecount ] )
      expectedmessagecount++

      if( "close" === d.action ) {
        expect( expectedmessagecount ).to.equal( 3 )
        channelb.close()
      }
    } )

    let channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointb.address().port, "codec": 8 } }, function( d ) {
      expect( d.action).to.not.equal( "telephone-event" )
      if( "close" === d.action ) channelc.close()
    } )

    let channelc = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointc.address().port, "codec": 97 } }, function( d ) {
      expect( d.action).to.not.equal( "telephone-event" )
      if( "close" === d.action ) done()
    } )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true
    expect( channela.mix( channelc ) ).to.be.true

    /* send a packet every 20mS x 50 */
    for( let i = 0;  i < 13; i ++ ) {
      sendpk( i, i*20, channela.local.port, endpointa )
    }

    senddtmf( 13, 12 * 160, 13*20, channela.local.port, endpointa, false, "4" )
    senddtmf( 14, 12 * 160, 14*20, channela.local.port, endpointa, false, "4" )
    senddtmf( 15, 12 * 160, 15*20, channela.local.port, endpointa, true, "4" )

    for( let i = 16;  i < 23; i ++ ) {
      sendpk( i, (i-3)*20, channela.local.port, endpointa )
    }

    senddtmf( 23, 22 * 160, 23*20, channela.local.port, endpointa, false, "5" )
    senddtmf( 24, 22 * 160, 24*20, channela.local.port, endpointa, false, "5" )
    senddtmf( 25, 22 * 160, 25*20, channela.local.port, endpointa, true, "5" )

    for( let i = 26;  i < 50; i ++ ) {
      sendpk( i, (i-6)*20, channela.local.port, endpointa )
    }

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 1200 ) } )

    channela.close()
    endpointa.close()
    endpointb.close()
    endpointc.close()

    expect( endpointapkcount ).to.be.within( 59, 70 )
    expect( endpointbpkcount ).to.be.within( 59, 70 )
    expect( endpointcpkcount ).to.be.within( 59, 70 )

    // 3 after we return to the event loop and enter the callback with close event.
    expect( expectedmessagecount ).to.equal( 2 )
    expect( dtmfapkcount ).to.equal( 0 )
    expect( dtmfbpkcount ).to.equal( 6 )
    expect( dtmfcpkcount ).to.equal( 6 )

    await finished

  } )

  it( `DTMF captured not working`, async function() {

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    var receviedpkcount = 0
    server.on( "message", function( msg, rinfo ) {
      receviedpkcount++
    } )

    this.timeout( 3000 )
    this.slow( 2500 )

    let done
    let finished = new Promise( ( r ) => { done = r } )

    server.bind()
    server.on( "listening", async function() {

      let ourport = server.address().port

      let expectedmessagecount = 0
      const expectedmessages = [
        { action: 'telephone-event', event: '3' },
        { action: 'close' }
      ]

      let channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {

        expect( d ).to.deep.include( expectedmessages[ expectedmessagecount ] )
        expectedmessagecount++

        if( "close" === d.action ) {
          server.close()
          done()
        }
      } )

      expect( channel.echo() ).to.be.true

      // Event "3"
      const dstport = channel.local.port
      sendpk( 948, 0, dstport, server, 0, 148480, 518218235 )
      sendpayload( 20, fromstr( "80 e5 03 b5 00 02 44 a0 1e e3 61 fb 03 0a 00 a0 4c d1" ), dstport, server )
      sendpk( 950, 40, dstport, server, 0, 148800, 518218235 )
      sendpayload( 60, fromstr( "80 65 03 b7 00 02 44 a0 1e e3 61 fb 03 0a 01 40 25 b8" ), dstport, server )
      sendpk( 952, 80, dstport, server, 0, 149120, 518218235 )
      sendpayload( 100, fromstr( "80 65 03 b9 00 02 44 a0 1e e3 61 fb 03 0a 01 e0 e8 81" ), dstport, server )
      sendpk( 954, 120, dstport, server, 0, 149440, 518218235 )
      sendpayload( 140, fromstr( "80 65 03 bb 00 02 44 a0 1e e3 61 fb 03 0a 02 80 e2 74" ), dstport, server )
      sendpk( 956, 160, dstport, server, 0, 149760, 518218235 )
      sendpayload( 180, fromstr( "80 65 03 bd 00 02 44 a0 1e e3 61 fb 03 0a 03 20 1f bb" ), dstport, server )
      sendpk( 958, 200, dstport, server, 0, 150080, 518218235 )
      sendpayload( 220, fromstr( "80 65 03 bf 00 02 44 a0 1e e3 61 fb 03 0a 03 c0 13 0c" ), dstport, server )
      sendpk( 960, 240, dstport, server, 0, 150400, 518218235 )
      sendpayload( 260, fromstr( "80 65 03 c1 00 02 44 a0 1e e3 61 fb 03 0a 04 60 2e bf" ), dstport, server )
      sendpk( 962, 280, dstport, server, 0, 150720, 518218235 )
      sendpayload( 300, fromstr( "80 65 03 c3 00 02 44 a0 1e e3 61 fb 03 8a 04 60 05 c1" ), dstport, server )
      sendpayload( 320 ,fromstr( "80 65 03 c4 00 02 44 a0 1e e3 61 fb 03 8a 04 60 bb 69" ), dstport, server )
      sendpk( 965, 340, dstport, server, 0, 151200, 518218235 )
      sendpayload( 360, fromstr( "80 65 03 c6 00 02 44 a0 1e e3 61 fb 03 8a 04 60 1e 27" ), dstport, server )
      sendpk( 967, 380, dstport, server, 0, 151520, 518218235 )
      sendpk( 968, 400, dstport, server, 0, 151680, 518218235 )
      sendpk( 969, 420, dstport, server, 0, 151840, 518218235 )
      sendpk( 970, 440, dstport, server, 0, 152000, 518218235 )

      setTimeout( () => channel.close(), 25*20 )
    } )

    await finished
  } )

  it( `DTMF PCAP playback test 1`, async function() {

    this.timeout( 21000 )
    this.slow( 20000 )

    let done
    let finished = new Promise( ( r ) => { done = r } )

    const receivedmessages = []

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    var receviedpkcount = 0
    server.on( "message", function( msg, rinfo ) {
      receviedpkcount++
    } )

    server.bind()
    server.on( "listening", async function() {

      let ourport = server.address().port
      let channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {
        receivedmessages.push( d )

        if( "close" === d.action ) {
          server.close()
          done()
        }
      } )
      setTimeout( () => channel.close(), 1000 * 17 )
      const dstport = channel.local.port

      channel.play( { "interupt":true, "files": [ { "wav": "test/interface/pcaps/180fa1ac-08e5-11ed-bd4d-02dba5b5aad6.wav" } ] } )

      let ourpcap = await pcap.readpcap( "test/interface/pcaps/dtmfcapture1.pcap" )

      const offset = 4700

      ourpcap.forEach( ( packet ) => {
        if( packet.ipv4 && packet.ipv4.udp && 10230 == packet.ipv4.udp.dstport ) {
          //console.dir( packet, { depth: null } )
          //console.log(packet.ipv4.udp.data.readUInt16BE( 2 ) )

          //const sn = packet.ipv4.udp.data.readUInt16BE( 2 )

          sendpayload( ( 1000 * packet.ts_sec_offset ) - offset, packet.ipv4.udp.data, dstport, server )
        }
      } )
    } )

    await finished

    const expectedmessages = [
      { action: 'play', event: 'start', reason: 'new' },
      { action: 'play', event: 'end', reason: 'telephone-event' },
      { action: 'telephone-event', event: '2' },
      { action: 'close' }
    ]

    expect( receivedmessages.length ).to.equal( 4 )
    expect( receivedmessages[ 0 ] ).to.deep.include( expectedmessages[ 0 ] )
    expect( receivedmessages[ 1 ] ).to.deep.include( expectedmessages[ 1 ] )
    expect( receivedmessages[ 2 ] ).to.deep.include( expectedmessages[ 2 ] )
    expect( receivedmessages[ 3 ] ).to.deep.include( expectedmessages[ 3 ] )

  } )

  it( `DTMF PCAP playback test 2`, async function() {

    this.timeout( 21000 )
    this.slow( 20000 )

    let done
    let finished = new Promise( ( r ) => { done = r } )

    const receivedmessages = []

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    var receviedpkcount = 0
    server.on( "message", function( msg, rinfo ) {
      receviedpkcount++
    } )

    server.bind()
    server.on( "listening", async function() {

      let ourport = server.address().port
      let channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {
        receivedmessages.push( d )

        if( "close" === d.action ) {
          server.close()
          done()
        }
      } )
      setTimeout( () => channel.close(), 1000 * 17 )
      const dstport = channel.local.port

      channel.play( { "interupt":true, "files": [ { "wav": "test/interface/pcaps/180fa1ac-08e5-11ed-bd4d-02dba5b5aad6.wav" } ] } )

      let ourpcap = await pcap.readpcap( "test/interface/pcaps/dtmf3presses.pcap" )

      const offset = 17 * 1000

      ourpcap.forEach( ( packet ) => {
        
        if( packet.ipv4 && packet.ipv4.udp && 10298 == packet.ipv4.udp.dstport ) {
          //console.dir( packet, { depth: null } )
          //console.log(packet.ipv4.udp.data.readUInt16BE( 2 ) )
          //const sn = packet.ipv4.udp.data.readUInt16BE( 2 )

          const sendat = ( 1000 * packet.ts_sec_offset ) - offset
          if ( sendat > 0 ) {
            //console.log(sn, sendat)
            sendpayload( sendat, packet.ipv4.udp.data, dstport, server )
          }
        }
      } )
    } )

    await finished

    const expectedmessages = [
      { action: 'play', event: 'start', reason: 'new' },
      { action: 'play', event: 'end', reason: 'telephone-event' },
      { action: 'telephone-event', event: '1' },
      { action: 'telephone-event', event: '1' },
      { action: 'telephone-event', event: '1' },
      { action: 'close' }
    ]

    expect( receivedmessages.length ).to.equal( 6 )
    expect( receivedmessages[ 0 ] ).to.deep.include( expectedmessages[ 0 ] )
    expect( receivedmessages[ 1 ] ).to.deep.include( expectedmessages[ 1 ] )
    expect( receivedmessages[ 2 ] ).to.deep.include( expectedmessages[ 2 ] )
    expect( receivedmessages[ 3 ] ).to.deep.include( expectedmessages[ 3 ] )
    expect( receivedmessages[ 4 ] ).to.deep.include( expectedmessages[ 4 ] )
    expect( receivedmessages[ 5 ] ).to.deep.include( expectedmessages[ 5 ] )

  } )
} )
