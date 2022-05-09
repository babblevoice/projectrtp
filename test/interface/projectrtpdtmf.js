
const expect = require( "chai" ).expect
const projectrtp = require( "../../index.js" ).projectrtp
const dgram = require( "dgram" )

/* helper functions */
function sendpk( sn, sendtime, dstport, server, pt = 0 ) {

  let ssrc = 25
  let pklength = 172

  return setTimeout( () => {

    let payload = Buffer.alloc( pklength - 12 ).fill( projectrtp.codecx.linear162pcmu( sn ) & 0xff )
    let subheader = Buffer.alloc( 10 )

    let ts = sn * 160

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
marker indicates start, endofevent marks the finish
*/
function senddtmf( sn, ts, sendtime, dstport, server, marker, endofevent, ev ) {

  return setTimeout( () => {
    let ssrc = 25
    let pklength = 58

    let header = Buffer.alloc( pklength )

    let pt = 101
    if( marker ) pt = pt | 0x80

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

      senddtmf( 23, 22 * 160, 23*20, channel.local.port, server, true, false, "4" )
      senddtmf( 24, 22 * 160, 24*20, channel.local.port, server, false, false, "4" )
      senddtmf( 25, 22 * 160, 25*20, channel.local.port, server, false, true, "4" )

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

    this.timeout( 3000 )
    this.slow( 2500 )

    clienta.bind()
    await new Promise( ( resolve, reject ) => { clienta.on( "listening", () => resolve()  ) } )
    clientb.bind()
    await new Promise( ( resolve, reject ) => { clientb.on( "listening", () => resolve()  ) } )

    let ouraport = clienta.address().port
    let ourbport = clientb.address().port

    let channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ouraport, "codec": 0 } }, function( d ) {
    } )

    let channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourbport, "codec": 0 } }, function( d ) {
    } )

    expect( channela.mix( channelb ) ).to.be.true

    /* send a packet every 20mS x 50 */
    for( let i = 0;  i < 50; i ++ ) {
      sendpk( i, i*20, channela.local.port, clienta )
    }

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 400 ) } )
    channela.dtmf( "*9F" )
    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 600 ) } )
    channela.close()
    channelb.close()

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

    let channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ouraport, "codec": 0 } }, function( d ) {
    } )

    let channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourbport, "codec": 0 } }, function( d ) {
    } )

    let channelc = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourcport, "codec": 0 } }, function( d ) {
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
    channelb.close()
    channelc.close()

    clienta.close()
    clientb.close()
    clientc.close()

    expect( dtmfpkcount ).to.equal( 5*3 )
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

      senddtmf( 13, 12 * 160, 13*20, channel.local.port, server, true, false, "4" )
      senddtmf( 14, 12 * 160, 14*20, channel.local.port, server, false, false, "4" )
      // Packet loss
      // senddtmf( 15, 12 * 160, 15*20, channel.port, server, false, true, "4" )

      for( let i = 16;  i < 23; i ++ ) {
        sendpk( i, (i-3)*20, channel.local.port, server )
      }

      senddtmf( 23, 22 * 160, 23*20, channel.local.port, server, true, false, "5" )
      senddtmf( 24, 22 * 160, 24*20, channel.local.port, server, false, false, "5" )
      senddtmf( 25, 22 * 160, 25*20, channel.local.port, server, false, true, "5" )

      for( let i = 26;  i < 33; i ++ ) {
        sendpk( i, (i-6)*20, channel.local.port, server )
      }

      setTimeout( () => channel.close(), 1000 )
    } )
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
    await new Promise( ( resolve, reject ) => { endpointa.on( "listening", function() { resolve() } ) } )

    endpointb.bind()
    await new Promise( ( resolve, reject ) => { endpointb.on( "listening", function() { resolve() } ) } )

    let expectedmessagecount = 0
    const expectedmessages = [
      { action: 'telephone-event', event: '4' },
      { action: 'telephone-event', event: '5' },
      { action: 'close' }
    ]

    let channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      expect( d ).to.deep.include( expectedmessages[ expectedmessagecount ] )
      expectedmessagecount++

      if( "close" === d.action ) {
        expect( expectedmessagecount ).to.equal( 3 )
      }
    } )

    let channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointb.address().port, "codec": 8 } }, function( d ) {

    } )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true

    /* send a packet every 20mS x 50 */
    for( let i = 0;  i < 13; i ++ ) {
      sendpk( i, i*20, channela.local.port, endpointa )
    }

    senddtmf( 13, 12 * 160, 13*20, channela.local.port, endpointa, true, false, "4" )
    senddtmf( 14, 12 * 160, 14*20, channela.local.port, endpointa, false, false, "4" )
    senddtmf( 15, 12 * 160, 15*20, channela.local.port, endpointa, false, true, "4" )

    for( let i = 16;  i < 23; i ++ ) {
      sendpk( i, (i-3)*20, channela.local.port, endpointa )
    }

    senddtmf( 23, 22 * 160, 23*20, channela.local.port, endpointa, true, false, "5" )
    senddtmf( 24, 22 * 160, 24*20, channela.local.port, endpointa, false, false, "5" )
    senddtmf( 25, 22 * 160, 25*20, channela.local.port, endpointa, false, true, "5" )

    for( let i = 26;  i < 50; i ++ ) {
      sendpk( i, (i-6)*20, channela.local.port, endpointa )
    }

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 1200 ) } )

    channela.close()
    channelb.close()
    endpointa.close()
    endpointb.close()

    expect( endpointapkcount ).to.be.within( 30, 51 )
    expect( endpointbpkcount ).to.be.within( 30, 51 )
    expect( dtmfpkcount ).to.equal( 6 )
    expect( expectedmessagecount ).to.equal( 2 )

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
    await new Promise( ( resolve, reject ) => { endpointa.on( "listening", function() { resolve() } ) } )

    endpointb.bind()
    await new Promise( ( resolve, reject ) => { endpointb.on( "listening", function() { resolve() } ) } )

    endpointc.bind()
    await new Promise( ( resolve, reject ) => { endpointc.on( "listening", function() { resolve() } ) } )

    let expectedmessagecount = 0
    const expectedmessages = [
      { action: 'telephone-event', event: '4' },
      { action: 'telephone-event', event: '5' },
      { action: 'close' }
    ]

    let channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      expect( d ).to.deep.include( expectedmessages[ expectedmessagecount ] )
      expectedmessagecount++

      if( "close" === d.action ) {
        expect( expectedmessagecount ).to.equal( 3 )
      }
    } )

    let channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointb.address().port, "codec": 8 } }, function( d ) {
      expect( d.action).to.not.equal( "telephone-event" )
    } )

    let channelc = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": endpointc.address().port, "codec": 97 } }, function( d ) {
      expect( d.action).to.not.equal( "telephone-event" )
    } )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true
    expect( channela.mix( channelc ) ).to.be.true

    /* send a packet every 20mS x 50 */
    for( let i = 0;  i < 13; i ++ ) {
      sendpk( i, i*20, channela.local.port, endpointa )
    }

    senddtmf( 13, 12 * 160, 13*20, channela.local.port, endpointa, true, false, "4" )
    senddtmf( 14, 12 * 160, 14*20, channela.local.port, endpointa, false, false, "4" )
    senddtmf( 15, 12 * 160, 15*20, channela.local.port, endpointa, false, true, "4" )

    for( let i = 16;  i < 23; i ++ ) {
      sendpk( i, (i-3)*20, channela.local.port, endpointa )
    }

    senddtmf( 23, 22 * 160, 23*20, channela.local.port, endpointa, true, false, "5" )
    senddtmf( 24, 22 * 160, 24*20, channela.local.port, endpointa, false, false, "5" )
    senddtmf( 25, 22 * 160, 25*20, channela.local.port, endpointa, false, true, "5" )

    for( let i = 26;  i < 50; i ++ ) {
      sendpk( i, (i-6)*20, channela.local.port, endpointa )
    }

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 1200 ) } )

    channela.close()
    channelb.close()
    channelc.close()
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

  } )
} )
