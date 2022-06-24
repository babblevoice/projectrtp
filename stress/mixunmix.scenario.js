
const projectrtp = require( "../index.js" ).projectrtp
const utils = require( "./utils.js" )
const expect = require( "chai" ).expect

/*
  clienta ---> channela ---> mix ---> channelb ---> clientb (and we echo back here)
*/

module.exports = async ( mstimeout ) => {

  utils.log( `Create 2 channels and mix then unmix for ${mstimeout} mS` )

  let acodec = utils.randcodec()
  let bcodec = utils.randcodec()

  const clienta = await projectrtp.openchannel( {}, ( d ) => {
    if( "close" === d.action ) {
      channela.close()
      utils.logclosechannel( `Mix 2 (clienta) for ${mstimeout} mS completed with reason '${d.reason}'`, d, mstimeout )
    }
  } )
  utils.lognewchannel()

  let channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": clienta.local.port, "codec": acodec } }, ( d ) => {
    if( "close" === d.action ) {
      utils.logclosechannel( `Mix 2 for ${mstimeout} mS completed with reason '${d.reason}'`, d, mstimeout )
    }
  } )
  clienta.remote( { "address": "localhost", "port": channela.local.port, "codec": acodec } )
  utils.lognewchannel()

  const clientb = await projectrtp.openchannel( {}, ( d ) => {
    if( "close" === d.action ) {
      channelb.close()
      utils.logclosechannel( `Mix 2 (clientb) for ${mstimeout} mS completed with reason '${d.reason}'`, d, mstimeout )
    }
  } )
  utils.lognewchannel()

  let channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": clientb.local.port, "codec": bcodec } }, ( d ) => {
    if( "close" === d.action ) {
      utils.logclosechannel( `Mix 2 for ${mstimeout} mS completed with reason '${d.reason}'`, d, mstimeout )
    }
  } )
  clientb.remote( { "address": "localhost", "port": channelb.local.port, "codec": bcodec } )
  utils.lognewchannel()

  expect( clientb.echo() ).to.be.true
  expect( channela.mix( channelb ) ).to.be.true
  expect( clienta.play( { "loop": true, "files": [ { "wav": "/tmp/ukringing.wav" } ] } ) ).to.be.true

  await new Promise( ( r ) => { setTimeout( () => r(), mstimeout / 2 ) } )

  clientb.close()
  channela.unmix()
  channela.echo()

  await new Promise( ( r ) => { setTimeout( () => r(), mstimeout / 2 ) } )

  clienta.close()

}
