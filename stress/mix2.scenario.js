
const projectrtp = require( "../index.js" ).projectrtp
const utils = require( "./utils.js" )
const expect = require( "chai" ).expect

/*
  clienta (play something) ---> channela ---> mix ---> channelb ---> clientb (and we echo back here)
*/

module.exports = async ( mstimeout ) => {

  utils.log( `Create 2 channels and mix for ${mstimeout} mS` )

  const acodec = utils.randcodec()
  const bcodec = utils.randcodec()

  const clienta = await projectrtp.openchannel( {}, ( d ) => {
    if( "close" === d.action ) {
      channela.close()
      utils.logclosechannel( `Mix 2 (clienta) for ${mstimeout} mS completed with reason '${d.reason}'.` +
        ` Expected number of packets: ${Math.round(mstimeout / 20)}, Received: ${d.stats.in["count"]},` +
        ` Score: ${(d.stats.in["count"] / mstimeout * 20).toFixed(2)}` )
    }
  } )
  utils.lognewchannel()

  const channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": clienta.local.port, "codec": acodec } }, ( d ) => {
    if( "close" === d.action ) {
      clientb.close()
      utils.logclosechannel( `Mix 2 (channela) for ${mstimeout} mS completed with reason '${d.reason}'.` +
        ` Expected number of packets: ${Math.round(mstimeout / 20)}, Received: ${d.stats.in["count"]},` +
        ` Score: ${(d.stats.in["count"] / mstimeout * 20).toFixed(2)}` )
    }
  } )

  clienta.remote( { "address": "localhost", "port": channela.local.port, "codec": acodec } )
  utils.lognewchannel()

  const clientb = await projectrtp.openchannel( {}, ( d ) => {
    if( "close" === d.action ) {
      channelb.close()
      utils.logclosechannel( `Mix 2 (clientb) for ${mstimeout} mS completed with reason '${d.reason}'.` +
        ` Expected number of packets: ${Math.round(mstimeout / 20)}, Received: ${d.stats.in["count"]},` +
        ` Score: ${(d.stats.in["count"] / mstimeout * 20).toFixed(2)}` )
    }
  } )
  utils.lognewchannel()

  const channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": clientb.local.port, "codec": bcodec } }, ( d ) => {
    if( "close" === d.action ) {
      utils.logclosechannel( `Mix 2 (channelb) for ${mstimeout} mS completed with reason '${d.reason}'.` +
        ` Expected number of packets: ${Math.round(mstimeout / 20)}, Received: ${d.stats.in["count"]},` +
        ` Score: ${(d.stats.in["count"] / mstimeout * 20).toFixed(2)}` )
    }
  } )
  clientb.remote( { "address": "localhost", "port": channelb.local.port, "codec": bcodec } )
  utils.lognewchannel()

  expect( channela.mix( channelb ) ).to.be.true
  expect( clienta.play( { "loop": true, "files": [ { "wav": "/tmp/ukringing.wav" } ] } ) ).to.be.true
  expect( clientb.echo() ).to.be.true

  /* Include some random DTMF */
  setTimeout( () => {
    clienta.dtmf( "*01239ABD" )
  }, Math.min( utils.between( 100, mstimeout ), 100 ) )

  setTimeout( () => {
    clienta.dtmf( "45678F#" )
  }, Math.min( utils.between( 100, mstimeout ), 100 ) )

  await new Promise( ( r ) => { setTimeout( () => r(), Math.max( mstimeout, 110 ) ) } )
  clienta.close()

}
