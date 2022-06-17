
const projectrtp = require( "../index.js" ).projectrtp
const utils = require( "./utils.js" )
const expect = require( "chai" ).expect

/*
  client[0] (play something) ---> channel[0] ---> mix ---> channel[i] ---> client[i] (and we echo back here from client[1])
*/

module.exports = async ( mstimeout ) => {

  max_channels = utils.between( 3, 6 )

  utils.log( `Create ${max_channels} channels and mix for ${mstimeout} mS` )

  const acodec = utils.randcodec()
  const clients = []
  const channels = []

  for ( var i = 0; i < max_channels; i++ )
  {
    clients.push( await projectrtp.openchannel( {}, ( d ) => {
      if( "close" === d.action ) {
        utils.logclosechannel( `Mix ${max_channels} (client${i}) for ${mstimeout} mS completed with reason '${d.reason}'` )
      }
    } ) )

    channels.push( await projectrtp.openchannel( { "remote": { "address": "localhost", "port": clients[i].local.port, "codec": acodec } }, ( d ) => {
      if( "close" === d.action ) {
        utils.logclosechannel( `Mix ${max_channels} (client${i}) for ${mstimeout} mS completed with reason '${d.reason}'` )
      }
    } ) )
    utils.lognewchannel()

    clients[i].remote = { "address": "localhost", "port": channels[i].local.port, "codec": acodec }
    utils.lognewchannel()

  }

  for ( var i = 1; i < max_channels; i++ )
  {
    expect( channels[0].mix( channels[i] ) ).to.be.true
  }

  expect( clients[0].play( { "loop": true, "files": [ { "wav": "/tmp/ukringing.wav" } ] } ) ).to.be.true
  expect( clients[1].echo() ).to.be.true

  await new Promise( ( r ) => { setTimeout( () => r(), Math.max( mstimeout, 110 ) ) } )
  
  for ( var i = 1; i < max_channels; i++ )
  {
    channels[i].close()
    clients[i].close()
  }
}
