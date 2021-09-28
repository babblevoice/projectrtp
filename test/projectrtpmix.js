


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
    if( undefined != data ) {
      let start = sn * 160
      let end = start + 160
      payload = data.subarray( start, end )
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

    let channela = projectrtp.rtpchannel.create( { "target": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {

      }
    } )

    let channelb = projectrtp.rtpchannel.create( { "target": { "address": "localhost", "port": endpointb.address().port, "codec": 0 } }, function( d ) {
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

    expect( endpointapkcount ).to.equal( 50 )

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

    let channela = projectrtp.rtpchannel.create( { "target": { "address": "localhost", "port": endpointa.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {

      }
    } )

    let channelb = projectrtp.rtpchannel.create( { "target": { "address": "localhost", "port": endpointb.address().port, "codec": 0 } }, function( d ) {
      if( "close" === d.action ) {

      }
    } )

    let channelc = projectrtp.rtpchannel.create( { "target": { "address": "localhost", "port": endpointc.address().port, "codec": 0 } }, function( d ) {
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
} )
