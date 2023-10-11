
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
  const retval = []
  str.split( " " ).forEach( v => retval.push ( parseInt( v, 16 ) ) )
  return Buffer.from( retval )
}

function sendpayload( sendtime, pk, dstport, server ) {
  return setTimeout( () => {
    server.send( pk, dstport, "localhost" )
  }, sendtime )
}

/* helper functions */
function sendpk( sn, ts, sendtime, dstport, server, pt = 0, ssrc ) {

  if( !ssrc ) ssrc = 25
  const pklength = 172

  return setTimeout( () => {

    const payload = Buffer.alloc( pklength - 12 ).fill( projectrtp.codecx.linear162pcmu( sn ) & 0xff )
    const subheader = Buffer.alloc( 10 )

    subheader.writeUInt8( pt, 1 ) // payload type
    subheader.writeUInt16BE( ( sn ) % ( 2**16 ) )
    subheader.writeUInt32BE( ts, 2 )
    subheader.writeUInt32BE( ssrc, 6 )

    const rtppacket = Buffer.concat( [
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
    const ssrc = 25
    const pklength = 58

    const header = Buffer.alloc( pklength )

    const pt = 101

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

  it( "Send 2833 DTMF and check event", function( done ) {

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    server.on( "message", function() {} )

    this.timeout( 3000 )
    this.slow( 2500 )

    server.bind()
    server.on( "listening", async function() {

      const ourport = server.address().port

      let expectedmessagecount = 0
      const expectedmessages = [
        { action: "telephone-event", event: "4" },
        { action: "close" }
      ]

      const channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {

        expect( d ).to.deep.include( expectedmessages[ expectedmessagecount ] )
        expectedmessagecount++

        if( "close" === d.action ) {
          server.close()
          done()
        }
      } )

      expect( channel.echo() ).to.be.true

      /* send a packet every 20mS x 50 */
      for( let i = 0;  23 > i; i ++ ) {
        sendpk( i, i*160, i*20, channel.local.port, server )
      }

      senddtmf( 23, 22*160, 23*20, channel.local.port, server, false, "4" )
      senddtmf( 24, 22*160, 24*20, channel.local.port, server, false, "4" )
      senddtmf( 25, 22*160, 25*20, channel.local.port, server, true, "4" )

      for( let i = 26;  40 > i; i ++ ) {
        sendpk( i, i*160, (i-3)*20, channel.local.port, server )
      }

      setTimeout( () => channel.close(), 1000 )
    } )
  } )


  it( "single channel and request rtp server to send 2833", function( done ) {

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    let dtmfpkcount = 0
    server.on( "message", function( msg ) {
      if( 101 == ( 0x7f & msg [ 1 ] ) ) {
        dtmfpkcount++
      } else {
        expect( msg.length ).to.equal( 172 )
        expect( 0x7f & msg [ 1 ] ).to.equal( 0 )
      }
    } )

    this.timeout( 3000 )
    this.slow( 2500 )

    server.bind()
    server.on( "listening", async function() {

      const ourport = server.address().port

      let expectedmessagecount = 0
      const expectedmessages = [
        { action: "close" }
      ]

      const channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {

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
      for( let i = 0;  50 > i; i ++ ) {
        sendpk( i, i*160, i*20, channel.local.port, server )
      }

      setTimeout( () => channel.dtmf( "#1" ), 400 )
      setTimeout( () => channel.close(), 1000 )
    } )
  } )


  it( "2 channels mixing and request rtp server to send 2833 to one", async function() {

    /* create our RTP/UDP endpoint */
    const clienta = dgram.createSocket( "udp4" )
    const clientb = dgram.createSocket( "udp4" )

    let dtmfpkcount = 0
    clienta.on( "message", function( msg ) {
      if( 101 == ( 0x7f & msg [ 1 ] ) ) {
        dtmfpkcount++
      } else {
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

    const ouraport = clienta.address().port
    const ourbport = clientb.address().port

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ouraport, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) channelb.close()
    } )

    const channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourbport, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) done()
    } )

    expect( channela.mix( channelb ) ).to.be.true

    /* send a packet every 20mS x 70 */
    for( let i = 0;  50 > i; i ++ ) {
      sendpk( i, i*160, i*20, channela.local.port, clienta )
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

  it( "3 channels mixing and request rtp server to send 2833 to one", async function() {

    /* create our RTP/UDP endpoint */
    const clienta = dgram.createSocket( "udp4" )
    const clientb = dgram.createSocket( "udp4" )
    const clientc = dgram.createSocket( "udp4" )

    let dtmfpkcount = 0
    clienta.on( "message", function( msg ) {
      if( 101 == ( 0x7f & msg [ 1 ] ) ) {
        dtmfpkcount++
      } else {
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

    clientc.on( "message", function( msg ) {
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
    clientc.bind()
    await new Promise( ( resolve ) => { clientc.on( "listening", () => resolve()  ) } )

    const ouraport = clienta.address().port
    const ourbport = clientb.address().port
    const ourcport = clientc.address().port

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ouraport, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) channelb.close()
    } )

    const channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourbport, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) channelc.close()
    } )

    const channelc = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourcport, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) done()
    } )

    expect( channela.mix( channelb ) ).to.be.true
    expect( channela.mix( channelc ) ).to.be.true

    /* send a packet every 20mS x 50 */
    for( let i = 0;  50 > i; i ++ ) {
      sendpk( i, i*160, i*20, channela.local.port, clienta )
    }

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 100 ) } )
    channela.dtmf( "*9ABD" )
    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 900 ) } )
    channela.close()

    clienta.close()
    clientb.close()
    clientc.close()

    expect( dtmfpkcount ).to.equal( 5*3 )

    await finished
  } )


  it( "Send multiple 2833 DTMF and check event", async function() {

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    server.on( "message", function() {} )

    this.timeout( 3000 )
    this.slow( 2500 )

    server.bind()
    await new Promise( ( resolve ) => { server.on( "listening", () => resolve() ) } )

    const ourport = server.address().port

    const receivedmessages = []
    const channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {
      receivedmessages.push( d )
      if( "close" === d.action ) {
        server.close()
      }
    } )

    expect( channel.echo() ).to.be.true

    /* send a packet every 20mS x 50 */
    for( let i = 0;  13 > i; i ++ ) {
      sendpk( i, i*160, i*20, channel.local.port, server )
    }

    /* DTMF every 50mS */
    senddtmf( 13, 12*160, 13*20, channel.local.port, server, false, "4" )
    sendpk( 14, 13*160, 13*20, channel.local.port, server )
    sendpk( 15, 14*160, 14*20, channel.local.port, server )
    senddtmf( 16, 14*160, (13*20)+50, channel.local.port, server, false, "4" )
    // Packet loss
    // senddtmf( 15, 12 * 160, 15*20, channel.port, server, true, "4" )

    for( let i = 17;  30 > i; i ++ ) {
      sendpk( i, (i-2)*160, (i-2)*20, channel.local.port, server )
    }

    senddtmf( 31, 29*160, 29*20, channel.local.port, server, false, "5" )
    sendpk( 32, 29*160, 29*20, channel.local.port, server )
    sendpk( 33, 30*160, 30*20, channel.local.port, server )
    senddtmf( 34, 30*160, (29*20)+50, channel.local.port, server, false, "5" )
    sendpk( 35, 31*160, 31*20, channel.local.port, server )
    sendpk( 36, 32*160, 32*20, channel.local.port, server )
    senddtmf( 37, 32*160, (29*20)+100, channel.local.port, server, true, "5" )

    for( let i = 37; 45 > i; i ++ ) {
      sendpk( i, i*160, (i-6)*20, channel.local.port, server )
    }

    setTimeout( () => channel.close(), 1100 )
    await new Promise( resolve => { server.on( "close", resolve ) } )

    expect( receivedmessages[ 0 ].action ).to.equal( "telephone-event" )
    expect( receivedmessages[ 0 ].event ).to.equal( "4" )
    expect( receivedmessages[ 1 ].action ).to.equal( "telephone-event" )
    expect( receivedmessages[ 1 ].event ).to.equal( "5" )
    expect( receivedmessages[ 2 ].action ).to.equal( "close" )

  } )

  it( "Lose end packet", async function() {

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    const receivedmessages = []

    let receviedpkcount = 0
    server.on( "message", function() {
      receviedpkcount++
    } )

    let done
    const finished = new Promise( r => done = r )

    this.timeout( 3000 )
    this.slow( 2500 )

    server.bind()
    server.on( "listening", async function() {

      const ourport = server.address().port
      const channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {
        receivedmessages.push( d )

        if( "close" === d.action ) {
          server.close()
          done()
        }
      } )

      expect( channel.echo() ).to.be.true

      /* send a packet every 20mS x 50 */
      sendpk( 0, 160, 0, channel.local.port, server )
      sendpk( 1, 1*160, 1*20, channel.local.port, server )
      sendpk( 2, 2*160, 2*20, channel.local.port, server )
      sendpk( 3, 3*160, 3*20, channel.local.port, server )
      sendpk( 4, 4*160, 4*20, channel.local.port, server )
      sendpk( 5, 5*160, 5*20, channel.local.port, server )
      sendpk( 6, 6*160, 6*20, channel.local.port, server )
      sendpk( 7, 7*160, 7*20, channel.local.port, server )
      sendpk( 8, 8*160, 8*20, channel.local.port, server )
      sendpk( 9, 9*160, 9*20, channel.local.port, server )
      sendpk( 10, 10*160, 10*20, channel.local.port, server )
      sendpk( 11, 11*160, 11*20, channel.local.port, server )
      sendpk( 12, 12*160, 12*20, channel.local.port, server )

      senddtmf( 13, 13*160, 13*20, channel.local.port, server, false, "4" )
      sendpk( 14, 13*160, 13*20, channel.local.port, server, 0 )
      sendpk( 15, 14*160, 14*20, channel.local.port, server, 0 )

      senddtmf( 16, (15*160)+10, (15*20)+10, channel.local.port, server, false, "4" )
      sendpk( 17, 15*160, 15*20, channel.local.port, server, 0 )
      sendpk( 18, 16*160, 16*20, channel.local.port, server, 0 )
      // Packet loss
      // senddtmf( 19, (16*160)+20, (16*20)+20, channel.local.port, server, true, "4" )

      sendpk( 20, 17*160, 17*20, channel.local.port, server, 0 )
      sendpk( 21, 18*160, 18*20, channel.local.port, server, 0 )
      sendpk( 22, 19*160, 19*20, channel.local.port, server, 0 )
      sendpk( 23, 20*160, 20*20, channel.local.port, server, 0 )
      sendpk( 24, 21*160, 21*20, channel.local.port, server, 0 )
      sendpk( 25, 22*160, 22*20, channel.local.port, server, 0 )
      sendpk( 26, 23*160, 23*20, channel.local.port, server, 0 )
      sendpk( 27, 24*160, 24*20, channel.local.port, server, 0 )
      sendpk( 28, 25*160, 25*20, channel.local.port, server, 0 )
      sendpk( 29, 26*160, 26*20, channel.local.port, server, 0 )
      sendpk( 30, 27*160, 27*20, channel.local.port, server, 0 )

      setTimeout( () => channel.close(), 1000 )

    } )

    await finished

    const expectedmessages = [
      { action: "telephone-event", event: "4" },
      { action: "close" }
    ]

    expect( receviedpkcount ).to.be.above( 15 )
    expect( receivedmessages.length ).to.equal( 2 )
    expect( receivedmessages[ 0 ] ).to.deep.include( expectedmessages[ 0 ] )
    expect( receivedmessages[ 1 ] ).to.deep.include( expectedmessages[ 1 ] )
  } )


  it( "mix 2 channels - pcmu <-> pcma and send DTMF", async function() {

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

    endpointa.on( "message", function( msg ) {
      endpointapkcount++
      expect( msg.length ).to.equal( 172 )
      expect( 0x7f & msg [ 1 ] ).to.equal( 0 )
    } )

    endpointb.on( "message", function( msg ) {
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
    const finished = new Promise( ( r ) => { done = r } )

    const channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      receivedmessages.push( d )

      if( "close" === d.action ) channelb.close()
    } )

    const channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointb.address().port, "codec": 8 } }, function( d ) {
      if( "close" === d.action ) done()
    } )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true

    /* send a packet every 20mS x 50 */
    sendpk( 0, 0, 0, channela.local.port, endpointa )
    sendpk( 1, 1*160, 1*20, channela.local.port, endpointa )
    sendpk( 2, 2*160, 2*20, channela.local.port, endpointa )
    sendpk( 3, 3*160, 3*20, channela.local.port, endpointa )
    sendpk( 4, 4*160, 4*20, channela.local.port, endpointa )
    sendpk( 5, 5*160, 5*20, channela.local.port, endpointa )
    sendpk( 6, 6*160, 6*20, channela.local.port, endpointa )
    sendpk( 7, 7*160, 7*20, channela.local.port, endpointa )
    sendpk( 8, 8*160, 8*20, channela.local.port, endpointa )
    sendpk( 9, 9*160, 9*20, channela.local.port, endpointa )
    sendpk( 10, 10*160, 10*20, channela.local.port, endpointa )
    sendpk( 11, 11*160, 11*20, channela.local.port, endpointa )
    sendpk( 12, 12*160, 12*20, channela.local.port, endpointa )

    senddtmf( 13, 13*160, 13*20, channela.local.port, endpointa, false, "4" )
    sendpk( 14, 13*160, 13*20, channela.local.port, endpointa, 0 )
    sendpk( 15, 14*160, 14*20, channela.local.port, endpointa, 0 )
    senddtmf( 16, (15*160)+10, (15*20)+10, channela.local.port, endpointa, false, "4" )
    sendpk( 17, 15*160, 15*20, channela.local.port, endpointa, 0 )
    sendpk( 18, 16*160, 16*20, channela.local.port, endpointa, 0 )
    senddtmf( 19, (17*160)+20, (17*20)+20, channela.local.port, endpointa, true, "4" )
    sendpk( 20, 17*160, 17*20, channela.local.port, endpointa, 0 )
    sendpk( 21, 18*160, 18*20, channela.local.port, endpointa, 0 )
    senddtmf( 22, (18*160)+30, (18*20)+30, channela.local.port, endpointa, true, "4" )
    sendpk( 23, 19*160, 19*20, channela.local.port, endpointa, 0 )
    sendpk( 24, 20*160, 20*20, channela.local.port, endpointa, 0 )
    sendpk( 25, 21*160, 21*20, channela.local.port, endpointa, 0 )
    sendpk( 26, 22*160, 22*20, channela.local.port, endpointa, 0 )
    sendpk( 27, 23*160, 23*20, channela.local.port, endpointa, 0 )
    sendpk( 28, 24*160, 24*20, channela.local.port, endpointa, 0 )
    sendpk( 29, 25*160, 25*20, channela.local.port, endpointa, 0 )

    senddtmf( 30, 26*160, 26*20, channela.local.port, endpointa, false, "5" )
    sendpk( 31, 26*160, 26*20, channela.local.port, endpointa, 0 )
    sendpk( 32, 27*160, 27*20, channela.local.port, endpointa, 0 )
    senddtmf( 33, (27*160)+10, (27*20)+10, channela.local.port, endpointa, false, "5" )
    sendpk( 34, 28*160, 28*20, channela.local.port, endpointa, 0 )
    sendpk( 35, 29*160, 28*20, channela.local.port, endpointa, 0 )
    senddtmf( 36, (28*160)+20, (28*20)+20, channela.local.port, endpointa, true, "5" )
    sendpk( 37, 30*160, 29*20, channela.local.port, endpointa, 0 )
    sendpk( 38, 31*160, 30*20, channela.local.port, endpointa, 0 )

    senddtmf( 39, (38*160)+30, (30*20)+30, channela.local.port, endpointa, true, "5" )
    sendpk( 40, 32*160, 31*20, channela.local.port, endpointa, 0 )
    sendpk( 41, 33*160, 32*20, channela.local.port, endpointa, 0 )

    sendpk( 42, 34*160, 33*20, channela.local.port, endpointa, 0 )
    sendpk( 43, 35*160, 34*20, channela.local.port, endpointa, 0 )
    sendpk( 44, 36*160, 35*20, channela.local.port, endpointa, 0 )
    sendpk( 45, 37*160, 36*20, channela.local.port, endpointa, 0 )
    sendpk( 46, 38*160, 37*20, channela.local.port, endpointa, 0 )
    sendpk( 47, 39*160, 38*20, channela.local.port, endpointa, 0 )
    sendpk( 48, 40*160, 39*20, channela.local.port, endpointa, 0 )
    sendpk( 49, 51*160, 40*20, channela.local.port, endpointa, 0 )

    await new Promise( ( r ) => { setTimeout( () => r(), 1400 ) } )

    channela.close()
    endpointa.close()
    endpointb.close()

    await finished

    expect( endpointapkcount ).to.be.within( 30, 51 )
    expect( endpointbpkcount ).to.be.within( 30, 51 )
    expect( dtmfpkcount ).to.be.within( 4, 8 )

    expect( receivedmessages.length ).to.equal( 5 )

    expect( receivedmessages[ 0 ].action ).to.equal( "mix" )
    expect( receivedmessages[ 1 ].action ).to.equal( "telephone-event" )
    expect( receivedmessages[ 2 ].action ).to.equal( "telephone-event" )
    expect( receivedmessages[ 3 ].action ).to.equal( "mix" )
    expect( receivedmessages[ 3 ].event ).to.equal( "finished" )
    expect( receivedmessages[ 4 ].action ).to.equal( "close" )

    expect( receivedmessages[ 0 ].event ).to.equal( "start" )
    expect( receivedmessages[ 1 ].event ).to.equal( "4" )
    expect( receivedmessages[ 2 ].event ).to.equal( "5" )
    expect( receivedmessages[ 3 ].event ).to.equal( "finished" )

  } )


  it( "mix 3 channels - pcmu <-> pcma and ilbc and send DTMF", async function() {

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

    endpointa.on( "message", function( msg ) {
      endpointapkcount++
      if( 101 == ( 0x7f & msg [ 1 ] ) ) {
        dtmfapkcount++
      } else {
        expect( msg.length ).to.equal( 172 )
        expect( 0x7f & msg [ 1 ] ).to.equal( 0 )
        endpointb.send( msg, channelb.local.port, "localhost" )
      }
    } )

    endpointb.on( "message", function( msg ) {
      endpointbpkcount++
      if( 101 == ( 0x7f & msg [ 1 ] ) ) {
        dtmfbpkcount++
      } else {
        expect( msg.length ).to.equal( 172 )
        expect( 0x7f & msg [ 1 ] ).to.equal( 8 )
        endpointb.send( msg, channelb.local.port, "localhost" )
      }
    } )

    endpointc.on( "message", function( msg ) {
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

    const receveiedmessages = []

    const channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      receveiedmessages.push( d )

      if( "close" === d.action ) channelb.close()
    } )

    const channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointb.address().port, "codec": 8 } }, function( d ) {
      expect( d.action).to.not.equal( "telephone-event" )
      if( "close" === d.action ) channelc.close()
    } )

    const channelc = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointc.address().port, "codec": 97 } }, function( d ) {
      expect( d.action).to.not.equal( "telephone-event" )
      if( "close" === d.action ) {
        endpointa.close()
        endpointb.close()
        endpointc.close()
      }
    } )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true
    expect( channela.mix( channelc ) ).to.be.true

    /* send a packet every 20mS x 50 */
    /* NO FOR LOOPS for explicit readablity of the test */
    sendpk( 0, 0, 0, channela.local.port, endpointa )
    sendpk( 1, 1*160, 20, channela.local.port, endpointa )
    sendpk( 2, 2*160, 2*20, channela.local.port, endpointa )
    sendpk( 3, 3*160, 3*20, channela.local.port, endpointa )
    sendpk( 4, 4*160, 4*20, channela.local.port, endpointa )
    sendpk( 5, 5*160, 5*20, channela.local.port, endpointa )
    sendpk( 6, 6*160, 6*20, channela.local.port, endpointa )
    sendpk( 7, 7*160, 7*20, channela.local.port, endpointa )
    sendpk( 8, 8*160, 8*20, channela.local.port, endpointa )
    sendpk( 9, 9*160, 9*20, channela.local.port, endpointa )
    sendpk( 10, 10*160, 10*20, channela.local.port, endpointa )
    sendpk( 11, 11*160, 11*20, channela.local.port, endpointa )
    sendpk( 12, 12*160, 12*20, channela.local.port, endpointa )

    /* rfc2833 - 3.6: An audio source SHOULD start transmitting event packets as soon as it
       recognizes an event and every 50 ms thereafter or the packet interval
       for the audio codec used for this session, if known. 
        This means our ts will not stay in sync with our sequence number - which increments 
        with every packet.
        senddtmf( sn, ts, sendtime, port, socket, endofevent, event )
        sendpk( sn, sendtime, port, socket, pt, ts, ssrc ) */
    senddtmf( 13, 13*160, 13*20, channela.local.port, endpointa, false, "4" )
    sendpk( 14, 13*160, 13*20, channela.local.port, endpointa, 0 )
    sendpk( 15, 14*160, 14*20, channela.local.port, endpointa, 0 )
    sendpk( 16, 15*160, 15*20, channela.local.port, endpointa, 0 )
    senddtmf( 17, (15*160), (13*20)+50, channela.local.port, endpointa, false, "4" )
    sendpk( 18, 16*160, 16*20, channela.local.port, endpointa, 0 )
    sendpk( 19, 17*160, 17*20, channela.local.port, endpointa, 0 )
    senddtmf( 20, (17*160), (13*20)+100, channela.local.port, endpointa, true, "4" )
    sendpk( 21, 18*160, 18*20, channela.local.port, endpointa, 0 )
    sendpk( 22, 19*160, 19*20, channela.local.port, endpointa, 0 )
    sendpk( 23, 20*160, 20*20, channela.local.port, endpointa, 0 )
    senddtmf( 24, (20*160), (13*20)+150, channela.local.port, endpointa, true, "4" )
    sendpk( 25, 21*160, 21*20, channela.local.port, endpointa, 0 )
    sendpk( 26, 22*160, 22*20, channela.local.port, endpointa, 0 )
    sendpk( 27, 23*160, 23*20, channela.local.port, endpointa, 0 )
    sendpk( 28, 24*160, 24*20, channela.local.port, endpointa, 0 )
    sendpk( 29, 25*160, 25*20, channela.local.port, endpointa, 0 )

    senddtmf( 30, 25*160, 25*20, channela.local.port, endpointa, false, "5" )
    sendpk( 31, 26*160, 26*20, channela.local.port, endpointa, 0 )
    sendpk( 32, 27*160, 27*20, channela.local.port, endpointa, 0 )
    senddtmf( 33, (27*160), (25*20)+50, channela.local.port, endpointa, false, "5" )
    sendpk( 34, 28*160, 28*20, channela.local.port, endpointa, 0 )
    sendpk( 35, 29*160, 29*20, channela.local.port, endpointa, 0 )
    senddtmf( 36, (29*160), (25*20)+100, channela.local.port, endpointa, true, "5" )
    sendpk( 37, 30*160, 30*20, channela.local.port, endpointa, 0 )
    sendpk( 38, 31*160, 31*20, channela.local.port, endpointa, 0 )
    sendpk( 39, 32*160, 32*20, channela.local.port, endpointa, 0 )
    senddtmf( 40, (32*160), (25*20)+150, channela.local.port, endpointa, true, "5" )
    sendpk( 41, 33*160, 33*20, channela.local.port, endpointa, 0 )
    sendpk( 42, 34*160, 34*20, channela.local.port, endpointa, 0 )
    sendpk( 43, 35*160, 35*20, channela.local.port, endpointa, 0 )
    sendpk( 44, 36*160, 36*20, channela.local.port, endpointa, 0 )
    sendpk( 45, 37*160, 37*20, channela.local.port, endpointa, 0 )
    sendpk( 46, 38*160, 38*20, channela.local.port, endpointa, 0 )
    sendpk( 47, 39*160, 39*20, channela.local.port, endpointa, 0 )
    sendpk( 48, 40*160, 40*20, channela.local.port, endpointa, 0 )
    sendpk( 49, 41*160, 41*20, channela.local.port, endpointa, 0 )
    sendpk( 50, 42*160, 42*20, channela.local.port, endpointa, 0 )
    sendpk( 51, 43*160, 43*20, channela.local.port, endpointa, 0 )
    sendpk( 52, 44*160, 44*20, channela.local.port, endpointa, 0 )
    sendpk( 53, 45*160, 45*20, channela.local.port, endpointa, 0 )
    sendpk( 54, 46*160, 46*20, channela.local.port, endpointa, 0 )
    sendpk( 55, 47*160, 47*20, channela.local.port, endpointa, 0 )
    sendpk( 56, 48*160, 48*20, channela.local.port, endpointa, 0 )
    sendpk( 57, 49*160, 49*20, channela.local.port, endpointa, 0 )
    sendpk( 58, 50*160, 50*20, channela.local.port, endpointa, 0 )
    sendpk( 59, 52*160, 51*20, channela.local.port, endpointa, 0 )

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 1200 ) } )

    channela.close()

    await Promise.all( [
      new Promise( resolve => { endpointa.on( "close", resolve ) } ),
      new Promise( resolve => { endpointb.on( "close", resolve ) } ),
      new Promise( resolve => { endpointc.on( "close", resolve ) } )
    ] )

    expect( endpointapkcount ).to.be.within( 59, 70 )
    expect( endpointbpkcount ).to.be.within( 59, 70 )
    expect( endpointcpkcount ).to.be.within( 59, 70 )

    // 3 after we return to the event loop and enter the callback with close event.
    expect( dtmfapkcount ).to.equal( 0 )
    expect( dtmfbpkcount ).to.be.within( 4, 8 )
    expect( dtmfcpkcount ).to.be.within( 4, 8 )

    expect( receveiedmessages[ 0 ].action ).to.equal( "mix" )
    expect( receveiedmessages[ 1 ].action ).to.equal( "mix" )
    expect( receveiedmessages[ 2 ].action ).to.equal( "telephone-event" )
    expect( receveiedmessages[ 3 ].action ).to.equal( "telephone-event" )
    expect( receveiedmessages[ 4 ].action ).to.equal( "mix" )
    expect( receveiedmessages[ 5 ].action ).to.equal( "close" )

    expect( receveiedmessages[ 0 ].event ).to.equal( "start" )
    expect( receveiedmessages[ 1 ].event ).to.equal( "start" )
    expect( receveiedmessages[ 2 ].event ).to.equal( "4" )
    expect( receveiedmessages[ 3 ].event ).to.equal( "5" )
    expect( receveiedmessages[ 4 ].event ).to.equal( "finished" )

  } )

  it( "DTMF captured not working", async function() {

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    server.on( "message", function() {} )

    this.timeout( 3000 )
    this.slow( 2500 )

    let done
    const finished = new Promise( ( r ) => { done = r } )

    server.bind()
    server.on( "listening", async function() {

      const ourport = server.address().port

      let expectedmessagecount = 0
      const expectedmessages = [
        { action: "telephone-event", event: "3" },
        { action: "close" }
      ]

      const channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {

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
      sendpk( 948, 148480, 0, dstport, server, 0, 518218235 )
      sendpayload( 20, fromstr( "80 e5 03 b5 00 02 44 a0 1e e3 61 fb 03 0a 00 a0 4c d1" ), dstport, server )
      sendpk( 950, 148800, 40, dstport, server, 0, 518218235 )
      sendpayload( 60, fromstr( "80 65 03 b7 00 02 44 a0 1e e3 61 fb 03 0a 01 40 25 b8" ), dstport, server )
      sendpk( 952, 149120, 80, dstport, server, 0, 518218235 )
      sendpayload( 100, fromstr( "80 65 03 b9 00 02 44 a0 1e e3 61 fb 03 0a 01 e0 e8 81" ), dstport, server )
      sendpk( 954, 149440,120, dstport, server, 0, 518218235 )
      sendpayload( 140, fromstr( "80 65 03 bb 00 02 44 a0 1e e3 61 fb 03 0a 02 80 e2 74" ), dstport, server )
      sendpk( 956, 149760, 160, dstport, server, 0, 518218235 )
      sendpayload( 180, fromstr( "80 65 03 bd 00 02 44 a0 1e e3 61 fb 03 0a 03 20 1f bb" ), dstport, server )
      sendpk( 958, 150080, 200, dstport, server, 0, 518218235 )
      sendpayload( 220, fromstr( "80 65 03 bf 00 02 44 a0 1e e3 61 fb 03 0a 03 c0 13 0c" ), dstport, server )
      sendpk( 960, 150400,240, dstport, server, 0, 518218235 )
      sendpayload( 260, fromstr( "80 65 03 c1 00 02 44 a0 1e e3 61 fb 03 0a 04 60 2e bf" ), dstport, server )
      sendpk( 962, 150720, 280, dstport, server, 0, 518218235 )
      sendpayload( 300, fromstr( "80 65 03 c3 00 02 44 a0 1e e3 61 fb 03 8a 04 60 05 c1" ), dstport, server )
      sendpayload( 320 ,fromstr( "80 65 03 c4 00 02 44 a0 1e e3 61 fb 03 8a 04 60 bb 69" ), dstport, server )
      sendpk( 965, 151200, 340, dstport, server, 0, 518218235 )
      sendpayload( 360, fromstr( "80 65 03 c6 00 02 44 a0 1e e3 61 fb 03 8a 04 60 1e 27" ), dstport, server )
      sendpk( 967, 151520, 380, dstport, server, 0, 518218235 )
      sendpk( 968, 151680, 400, dstport, server, 0, 518218235 )
      sendpk( 969, 151840, 420, dstport, server, 0, 518218235 )
      sendpk( 970, 152000, 440, dstport, server, 0, 518218235 )

      setTimeout( () => channel.close(), 25*20 )
    } )

    await finished
  } )

  it( "DTMF PCAP playback test 1", async function() {

    this.timeout( 21000 )
    this.slow( 20000 )

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const receivedmessages = []

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    server.on( "message", function() {} )

    server.bind()
    server.on( "listening", async function() {

      const ourport = server.address().port
      const channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {
        receivedmessages.push( d )

        if( "close" === d.action ) {
          server.close()
          done()
        }
      } )
      setTimeout( () => channel.close(), 1000 * 17 )
      const dstport = channel.local.port

      channel.play( { "interupt":true, "files": [ { "wav": "test/interface/pcaps/180fa1ac-08e5-11ed-bd4d-02dba5b5aad6.wav" } ] } )

      const ourpcap = await pcap.readpcap( "test/interface/pcaps/dtmfcapture1.pcap" )

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
      { action: "play", event: "start", reason: "new" },
      { action: "play", event: "end", reason: "telephone-event" },
      { action: "telephone-event", event: "2" },
      { action: "close" }
    ]

    expect( receivedmessages.length ).to.equal( 4 )
    expect( receivedmessages[ 0 ] ).to.deep.include( expectedmessages[ 0 ] )
    expect( receivedmessages[ 1 ] ).to.deep.include( expectedmessages[ 1 ] )
    expect( receivedmessages[ 2 ] ).to.deep.include( expectedmessages[ 2 ] )
    expect( receivedmessages[ 3 ] ).to.deep.include( expectedmessages[ 3 ] )

  } )

  it( "DTMF PCAP playback test 2", async function() {

    this.timeout( 21000 )
    this.slow( 20000 )

    let done
    const finished = new Promise( ( r ) => { done = r } )

    const receivedmessages = []

    /* create our RTP/UDP endpoint */
    const server = dgram.createSocket( "udp4" )
    server.on( "message", function() {} )

    server.bind()
    server.on( "listening", async function() {

      const ourport = server.address().port
      const channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, function( d ) {
        receivedmessages.push( d )

        if( "close" === d.action ) {
          server.close()
          done()
        }
      } )
      setTimeout( () => channel.close(), 1000 * 17 )
      const dstport = channel.local.port

      channel.play( { "interupt":true, "files": [ { "wav": "test/interface/pcaps/180fa1ac-08e5-11ed-bd4d-02dba5b5aad6.wav" } ] } )

      const ourpcap = await pcap.readpcap( "test/interface/pcaps/dtmf3presses.pcap" )

      const offset = 17 * 1000

      ourpcap.forEach( ( packet ) => {
        
        if( packet.ipv4 && packet.ipv4.udp && 10298 == packet.ipv4.udp.dstport ) {
          //console.dir( packet, { depth: null } )
          //console.log(packet.ipv4.udp.data.readUInt16BE( 2 ) )
          //const sn = packet.ipv4.udp.data.readUInt16BE( 2 )

          const sendat = ( 1000 * packet.ts_sec_offset ) - offset
          if ( 0 < sendat ) {
            //console.log(sn, sendat)
            sendpayload( sendat, packet.ipv4.udp.data, dstport, server )
          }
        }
      } )
    } )

    await finished

    const expectedmessages = [
      { action: "play", event: "start", reason: "new" },
      { action: "play", event: "end", reason: "telephone-event" },
      { action: "telephone-event", event: "1" },
      { action: "telephone-event", event: "1" },
      { action: "telephone-event", event: "1" },
      { action: "close" }
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
