
const projectrtp = require( "../index.js" ).projectrtp
const utils = require( "./utils.js" )
const expect = require( "chai" ).expect

/*
  client[i] (play something) ---> channel[i] ---> mix ---> channel[j] ---> client[j]
*/

module.exports = async ( mstimeout ) => {

  // Random number of channels between 3 and 6
  const max_channels = utils.between( 3, 6 )
  utils.log( `Create ${max_channels} channels and mix for ${mstimeout} mS` )

  const acodec = utils.randcodec()
  const clients = []
  const channels = []

  // First create clients/channels and set remote
  // Channels are internal and used for mixing, while as clients are remote nodes
  for ( var i = 0; i < max_channels; i++ )
  {
    clients.push( await projectrtp.openchannel( {}, ( d ) => {
      if( "close" === d.action ) {
        utils.logclosechannel( `Mix ${max_channels} (client) for ${mstimeout} mS completed with reason '${d.reason}'`, d, mstimeout )
      }
    } ) )

    channels.push( await projectrtp.openchannel( { "remote": { "address": "localhost", "port": clients[i].local.port, "codec": acodec } }, ( d ) => {
      if( "close" === d.action ) {
        utils.logclosechannel( `Mix ${max_channels} (channel) for ${mstimeout} mS completed with reason '${d.reason}'`, d, mstimeout )
      }
    } ) )
    utils.lognewchannel()

    clients[i].remote = { "address": "localhost", "port": channels[i].local.port, "codec": acodec }
    utils.lognewchannel()

  }

  // Mix channel[0] with every other channel
  for ( var i = 1; i < max_channels; i++ )
  {
    expect( channels[0].mix( channels[i] ) ).to.be.true
  }

  for ( var i = 0; i < max_channels; i++ )
  {
    expect( clients[i].play( { "loop": true, "files": [ { "wav": "/tmp/ukringing.wav" } ] } ) ).to.be.true
  }

  await new Promise( ( r ) => { setTimeout( () => r(), Math.max( mstimeout, 110 ) ) } )
  
  // Clean up
  for ( var i = 0; i < max_channels; i++ )
  {
    channels[i].close()
    clients[i].close()
  }
}
