
const projectrtp = require( "../index.js" ).projectrtp
const fs = require( "fs" )
const dgram = require( "dgram" )
const utils = require( "./utils.js" )
const expect = require( "chai" ).expect

const possiblecodecs = [ 0, 8, 9, 97 ]
/*
  clienta ---> channela ---> mix ---> channelb ---> clientb (and we echo back here)
*/

module.exports = async ( packets ) => {

  utils.log( `Create 2 channels and mix ${packets} packets then unmix` )

  let bcodec = possiblecodecs[ utils.between( 0, possiblecodecs.length ) ]

  const clienta = dgram.createSocket( "udp4" )
  clienta.bind()
  await new Promise( ( resolve, reject ) => { clienta.on( "listening", function() { resolve() } ) } )
  const clientaport = clienta.address().port

  const clientb = dgram.createSocket( "udp4" )
  clientb.bind()
  await new Promise( ( resolve, reject ) => { clientb.on( "listening", function() { resolve() } ) } )
  const clientbport = clientb.address().port

  let channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": clientaport, "codec": 0 } }, function( d ) {
    if( "close" === d.action ) {
      utils.cancelremainingscheduled( clienta )
      clienta.close()
      utils.logclosechannel( `Mix 2 for ${packets} packets completed with reason '${d.reason}'` )
    }
  } )
  utils.lognewchannel()

  let channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": clientbport, "codec": bcodec } }, function( d ) {
    if( "close" === d.action ) {
      utils.cancelremainingscheduled( clientb )
      clientb.close()
      utils.logclosechannel( `Mix 2 for ${packets} packets completed with reason '${d.reason}'` )
    }
  } )
  utils.lognewchannel()

  /* echo back */
  clientb.on( "message", function( msg, rinfo ) {
    clientb.send( msg, channelb.local.port, "localhost" )
  } )

  expect( channela.mix( channelb ) ).to.be.true

  let payload = Buffer.alloc( 172 - 12 ).fill( projectrtp.codecx.linear162pcmu( 0 ) )
  let ssrc = utils.between( 10, 100 )
  /* send a packet every 20mS x 50 */
  for( let i = 0;  i < packets; i ++ ) {
    utils.sendpk( i, i * 20, channela.local.port, clienta, ssrc, payload )
  }

  await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), packets / 2 * 20 ) } )

  channelb.close()
  channela.echo()

  await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), packets / 2 * 20 ) } )

  channela.close()

}
