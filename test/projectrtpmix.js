


const expect = require( "chai" ).expect
const fs = require( "fs" )
const fspromises = fs.promises

const dgram = require( "dgram" )

let projectrtp
if( "debug" === process.env.build ) {
  projectrtp = require( "../src/build/Debug/projectrtp" )
} else {
  projectrtp = require( "../src/build/Release/projectrtp" )
}

function sendpk( sn, sendtime, dstport, server, data = undefined ) {

  let ssrc = 25
  let pklength = 172

  return setTimeout( () => {

    let payload
    if( undefined !== data ) {
      payload = data
    } else {
      payload = Buffer.alloc( pklength - 12 ).fill( projectrtp.codecx.linear162pcmu( sn ) & 0xff )
    }

    let subheader = Buffer.alloc( 10 )

    let ts = sn * 160

    subheader.writeUInt16BE( ( sn + 100 ) % ( 2**16 ) /* just some offset */ )
    subheader.writeUInt32BE( ts, 2 )
    subheader.writeUInt32BE( ssrc, 6 )

    let rtppacket = Buffer.concat( [
      Buffer.from( [ 0x80, 0x00 ] ),
      subheader,
      payload ] )

    server.send( rtppacket, dstport, "localhost" )
  }, sendtime * 20 )
}


describe( "channel mix", function() {

  it( `basic mix 2 channels`, async function() {

    this.timeout( 3000 )
    this.slow( 2000 )

    const endpointa = dgram.createSocket( "udp4" )
    const endpointb = dgram.createSocket( "udp4" )

    let endpointapkcount = 0

    endpointa.on( "message", function( msg, rinfo ) {
      endpointapkcount++
    } )

    endpointb.on( "message", function( msg, rinfo ) {
    } )

    endpointa.bind()
    await new Promise( ( resolve, reject ) => { endpointa.on( "listening", function() { resolve() } ) } )

    endpointb.bind()
    await new Promise( ( resolve, reject ) => { endpointb.on( "listening", function() { resolve() } ) } )

    let channela = projectrtp.openchannel( { "target": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {

      }
    } )

    let channelb = projectrtp.openchannel( { "target": { "address": "localhost", "port": endpointb.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {

      }
    } )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true

    /* Now, when we send UDP on endpointb it  passes through our mix then arrives at endpointa */
    for( let i = 0;  i < 50; i ++ ) {
      sendpk( i, i, channelb.port, endpointb )
    }

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 1200 ) } )

    channela.close()
    channelb.close()
    endpointa.close()
    endpointb.close()

    expect( endpointapkcount ).to.be.above( 48 )

  } )

  it( `mix 3 channels - 1 writer 3 readers`, async function() {

    this.timeout( 3000 )
    this.slow( 2000 )

    const endpointa = dgram.createSocket( "udp4" )
    const endpointb = dgram.createSocket( "udp4" )
    const endpointc = dgram.createSocket( "udp4" )

    let endpointapkcount = 0
    let endpointbpkcount = 0

    endpointa.on( "message", function( msg, rinfo ) {
      endpointapkcount++
      expect( projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ).to.be.oneOf([0 , 8 ] )
    } )

    endpointb.on( "message", function( msg, rinfo ) {
      endpointbpkcount++
      expect( projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ).to.be.oneOf([0 , 8 ] )
    } )

    endpointc.on( "message", function( msg, rinfo ) {
      expect( projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ).to.be.oneOf([0 , 8 ] )
    } )

    endpointa.bind()
    await new Promise( ( resolve, reject ) => { endpointa.on( "listening", function() { resolve() } ) } )

    endpointb.bind()
    await new Promise( ( resolve, reject ) => { endpointb.on( "listening", function() { resolve() } ) } )

    endpointc.bind()
    await new Promise( ( resolve, reject ) => { endpointc.on( "listening", function() { resolve() } ) } )

    let channela = projectrtp.openchannel( { "target": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {

      }
    } )

    let channelb = projectrtp.openchannel( { "target": { "address": "localhost", "port": endpointb.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {

      }
    } )

    let channelc = projectrtp.openchannel( { "target": { "address": "localhost", "port": endpointc.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {
      }
    } )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true
    expect( channela.mix( channelc ) ).to.be.true

    /* Now, when we send UDP on endpointb it  passes through our mix then arrives at endpointa */
    for( let i = 0;  i < 50; i ++ ) {
      sendpk( i, i, channelc.port, endpointc, Buffer.alloc( 160 ).fill( projectrtp.codecx.linear162pcmu( 8 ) ) )
    }

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 1200 ) } )

    channela.close()
    channelb.close()
    channelc.close()
    endpointa.close()
    endpointb.close()
    endpointc.close()

    /* This value is based on timeing so may vary very slightly */
    expect( endpointapkcount ).to.be.within( 59, 61 )
    expect( endpointbpkcount ).to.be.within( 59, 61 )

  } )

  it( `mix 3 channels - 1 writer 1 readers (2 silenced)`, async function() {

    this.timeout( 3000 )
    this.slow( 2000 )

    const endpointa = dgram.createSocket( "udp4" )
    const endpointb = dgram.createSocket( "udp4" )
    const endpointc = dgram.createSocket( "udp4" )

    let endpointapkcount = 0
    let endpointbpkcount = 0
    let endpointcpkcount = 0

    endpointa.on( "message", function( msg, rinfo ) {
      endpointapkcount++
      expect( projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ).to.be.oneOf([0 , 8 ] )
    } )

    endpointb.on( "message", function( msg, rinfo ) {
      endpointbpkcount++
      expect( projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ).to.be.oneOf([0 , 8 ] )
    } )

    endpointc.on( "message", function( msg, rinfo ) {
      endpointcpkcount++
      expect( projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ).to.be.oneOf([0 , 8 ] )
    } )

    endpointa.bind()
    await new Promise( ( resolve, reject ) => { endpointa.on( "listening", function() { resolve() } ) } )

    endpointb.bind()
    await new Promise( ( resolve, reject ) => { endpointb.on( "listening", function() { resolve() } ) } )

    endpointc.bind()
    await new Promise( ( resolve, reject ) => { endpointc.on( "listening", function() { resolve() } ) } )

    let channela = projectrtp.openchannel( { "direction": { "send": false }, "target": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {

      }
    } )

    let channelb = projectrtp.openchannel( { "direction": { "send": false }, "target": { "address": "localhost", "port": endpointb.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {

      }
    } )

    let channelc = projectrtp.openchannel( { "target": { "address": "localhost", "port": endpointc.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {
      }
    } )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true
    expect( channela.mix( channelc ) ).to.be.true

    /* Now, when we send UDP on endpointb it  passes through our mix then arrives at endpointa */
    for( let i = 0;  i < 50; i ++ ) {
      sendpk( i, i, channelc.port, endpointc, Buffer.alloc( 160 ).fill( projectrtp.codecx.linear162pcmu( 8 ) ) )
    }

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 1200 ) } )

    channela.close()
    channelb.close()
    channelc.close()
    endpointa.close()
    endpointb.close()
    endpointc.close()

    expect( endpointapkcount ).to.be.equal( 0 )
    expect( endpointbpkcount ).to.be.equal( 0 )
    expect( endpointcpkcount ).to.be.within( 59, 61 )

  } )

  it( `mix 3 channels - 1 writer 3 recevers but writer recv=false`, async function() {

    this.timeout( 3000 )
    this.slow( 2000 )

    const endpointa = dgram.createSocket( "udp4" )
    const endpointb = dgram.createSocket( "udp4" )
    const endpointc = dgram.createSocket( "udp4" )

    let endpointapkcount = 0
    let endpointbpkcount = 0
    let endpointcpkcount = 0

    endpointa.on( "message", function( msg, rinfo ) {
      endpointapkcount++
      expect( projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ).to.equal( 0 )
    } )

    endpointb.on( "message", function( msg, rinfo ) {
      endpointbpkcount++
      expect( projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ).to.equal( 0 )
    } )

    endpointc.on( "message", function( msg, rinfo ) {
      endpointcpkcount++
      expect( projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ).to.equal( 0 )
    } )

    endpointa.bind()
    await new Promise( ( resolve, reject ) => { endpointa.on( "listening", function() { resolve() } ) } )

    endpointb.bind()
    await new Promise( ( resolve, reject ) => { endpointb.on( "listening", function() { resolve() } ) } )

    endpointc.bind()
    await new Promise( ( resolve, reject ) => { endpointc.on( "listening", function() { resolve() } ) } )

    let channela = projectrtp.openchannel( { "target": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {

      }
    } )

    let channelb = projectrtp.openchannel( { "target": { "address": "localhost", "port": endpointb.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {

      }
    } )

    let channelc = projectrtp.openchannel( { "direction": { "recv": false }, "target": { "address": "localhost", "port": endpointc.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {
      }
    } )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true
    expect( channela.mix( channelc ) ).to.be.true

    /* Send data - which should be ignored but receviers should recevie silence (in payload) */
    for( let i = 0;  i < 50; i ++ ) {
      sendpk( i, i, channelc.port, endpointc )
    }

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 1200 ) } )

    channela.close()
    channelb.close()
    channelc.close()
    endpointa.close()
    endpointb.close()
    endpointc.close()

    expect( endpointapkcount ).to.be.within( 59, 61 )
    expect( endpointbpkcount ).to.be.within( 59, 61 )
    expect( endpointcpkcount ).to.be.within( 59, 61 )

  } )


  it( `mix 3 channels - 1 writer 3 recevers but writer delayed recv=false`, async function() {

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

    endpointa.on( "message", function( msg, rinfo ) {
      if( 0 == projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ) {
        endpointapkcountzero++
      } else {
        endpointapkcountnotzero++
      }
    } )

    endpointb.on( "message", function( msg, rinfo ) {
      if( 0 == projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ) {
        endpointbpkcountzero++
      } else {
        endpointbpkcountnotzero++
      }
    } )

    endpointc.on( "message", function( msg, rinfo ) {
      if( 0 == projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ) {
        endpointcpkcountzero++
      } else {
        endpointcpkcountnotzero++
      }
    } )

    endpointa.bind()
    await new Promise( ( resolve, reject ) => { endpointa.on( "listening", function() { resolve() } ) } )

    endpointb.bind()
    await new Promise( ( resolve, reject ) => { endpointb.on( "listening", function() { resolve() } ) } )

    endpointc.bind()
    await new Promise( ( resolve, reject ) => { endpointc.on( "listening", function() { resolve() } ) } )

    let channela = projectrtp.openchannel( { "target": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {

      }
    } )

    let channelb = projectrtp.openchannel( { "target": { "address": "localhost", "port": endpointb.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {

      }
    } )

    let channelc = projectrtp.openchannel( { "target": { "address": "localhost", "port": endpointc.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {
      }
    } )

    setTimeout( () => expect( channelc.direction( { "recv": false } ) ).to.be.true , 400 )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true
    expect( channela.mix( channelc ) ).to.be.true

    /* Send data - which should be ignored but receviers should recevie silence (in payload) */
    for( let i = 0;  i < 50; i ++ ) {
      sendpk( i, i, channelc.port, endpointc, Buffer.alloc( 160 ).fill( projectrtp.codecx.linear162pcmu( 8 ) ) )
    }

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 1200 ) } )

    channela.close()
    channelb.close()
    channelc.close()
    endpointa.close()
    endpointb.close()
    endpointc.close()

    expect( endpointapkcountzero ).to.be.within( 49, 51 )
    expect( endpointbpkcountzero ).to.be.within( 49, 51 )
    expect( endpointcpkcountzero ).to.be.within( 59, 61 )
    expect( endpointapkcountnotzero ).to.be.within( 8, 12 )
    expect( endpointbpkcountnotzero ).to.be.within( 8, 12 )
    expect( endpointcpkcountnotzero ).to.equal( 0 )

  } )

  it( `mix 3 channels - 1 writer 1 readers (2 silenced but delayed)`, async function() {

    this.timeout( 3000 )
    this.slow( 2000 )

    const endpointa = dgram.createSocket( "udp4" )
    const endpointb = dgram.createSocket( "udp4" )
    const endpointc = dgram.createSocket( "udp4" )

    let endpointapkcount = 0
    let endpointbpkcount = 0
    let endpointcpkcount = 0

    endpointa.on( "message", function( msg, rinfo ) {
      endpointapkcount++
      expect( projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ).to.be.oneOf([0 , 8 ] )
    } )

    endpointb.on( "message", function( msg, rinfo ) {
      endpointbpkcount++
      expect( projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ).to.be.oneOf([0 , 8 ] )
    } )

    endpointc.on( "message", function( msg, rinfo ) {
      endpointcpkcount++
      expect( projectrtp.codecx.pcmu2linear16( msg[ 30 ] ) ).to.be.oneOf([0 , 8 ] )
    } )

    endpointa.bind()
    await new Promise( ( resolve, reject ) => { endpointa.on( "listening", function() { resolve() } ) } )

    endpointb.bind()
    await new Promise( ( resolve, reject ) => { endpointb.on( "listening", function() { resolve() } ) } )

    endpointc.bind()
    await new Promise( ( resolve, reject ) => { endpointc.on( "listening", function() { resolve() } ) } )

    let channela = projectrtp.openchannel( { "target": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {

      }
    } )

    let channelb = projectrtp.openchannel( { "target": { "address": "localhost", "port": endpointb.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {

      }
    } )

    let channelc = projectrtp.openchannel( { "target": { "address": "localhost", "port": endpointc.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {
      }
    } )

    setTimeout( () => expect( channela.direction( { "send": false } ) ).to.be.true , 200 )
    setTimeout( () => expect( channelb.direction( { "send": false } ) ).to.be.true , 200 )

    /* mix */
    expect( channela.mix( channelb ) ).to.be.true
    expect( channela.mix( channelc ) ).to.be.true

    /* Now, when we send UDP on endpointb it  passes through our mix then arrives at endpointa */
    for( let i = 0;  i < 50; i ++ ) {
      sendpk( i, i, channelc.port, endpointc, Buffer.alloc( 160 ).fill( projectrtp.codecx.linear162pcmu( 8 ) ) )
    }

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 1200 ) } )

    channela.close()
    channelb.close()
    channelc.close()
    endpointa.close()
    endpointb.close()
    endpointc.close()

    expect( endpointapkcount ).to.be.at.most( 15 )
    expect( endpointbpkcount ).to.be.at.most( 15 )
    expect( endpointcpkcount ).to.be.within( 59, 61 )

  } )
} )
