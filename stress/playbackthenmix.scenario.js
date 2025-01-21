
const projectrtp = require( "../index.js" ).projectrtp
const utils = require( "./utils.js" )
const expect = require( "chai" ).expect

/**
 * clienta (echo) ---> channela (play/mix)
 * @param { number } mstimeout 
 */
module.exports = async ( mstimeout ) => {

  const acodec = utils.randcodec()
  const bcodec = utils.randcodec()

  utils.log( `Starting playback then mix for ${mstimeout} mS` )

  const clienta = await projectrtp.openchannel( {}, ( d ) => {
    if( "close" === d.action ) {
      channela.close()
      utils.logclosechannel( `Mix 2 after play (clienta) for ${mstimeout} mS completed with reason '${d.reason}'`, d, mstimeout )
    }
  } )
  utils.lognewchannel()

  const channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": clienta.local.port, "codec": acodec } }, async ( d ) => {
    if( "play" === d.action && "end" === d.event && "channelmixing" === d.reason ) {
      utils.log( "Channel stopped playback because channel mixing" )
    } else if( "close" === d.action ) {
      utils.logclosechannel( `Playback then mix for ${mstimeout} mS completed with reason '${d.reason}'`, d, mstimeout )
    }
  } )

  clienta.remote( { "address": "localhost", "port": channela.local.port, "codec": acodec } )
  clienta.echo()
  utils.lognewchannel()

  channela.play( {
    "loop": true,
    "files": [
      { "wav": "/tmp/ukringing.wav" }
    ]
  } )

  await utils.waitbetween( 100, 1000 )
  
  const clientb = await projectrtp.openchannel( {}, ( d ) => {
    if( "close" === d.action ) {
    channelb.close()
    utils.logclosechannel( `Mix 2 after play (clientb) for ${mstimeout} mS completed with reason '${d.reason}'`, d, mstimeout )
    }
  } )
  utils.lognewchannel()

  const channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": clientb.local.port, "codec": bcodec } }, ( d ) => {
      if( "close" === d.action ) {
      utils.logclosechannel( `Mix 2 after play (channelb) for ${mstimeout} mS completed with reason '${d.reason}'`, d, mstimeout )
      }
  } )
  clientb.remote( { "address": "localhost", "port": channelb.local.port, "codec": bcodec } )
  utils.lognewchannel()

  expect( await channela.mix( channelb ) ).to.be.true
  expect( clienta.play( { "loop": true, "files": [ { "wav": "/tmp/ukringing.wav" } ] } ) ).to.be.true
  expect( clientb.echo() ).to.be.true

  await new Promise( ( r ) => { setTimeout( () => r(), Math.max( mstimeout, 110 ) ) } )
  clienta.close()
  clientb.close()

}
