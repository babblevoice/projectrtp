
const projectrtp = require( "../index.js" ).projectrtp
const fs = require( "fs" )
const utils = require( "./utils.js" )

/*
  clienta (play) ---> channela (echo/record)
*/

module.exports = async ( mstimeout ) => {

  const acodec = utils.randcodec()

  utils.log( `Starting record with power detection for ${mstimeout} mS` )

  const recording = utils.mktempwav()
  const powerrecording = utils.mktempwav()

  const clienta = await projectrtp.openchannel( {}, ( d ) => {
    if( "close" === d.action ) {
      channela.close()
      utils.logclosechannel( `Mix 2 (clienta) for ${mstimeout} mS completed with reason '${d.reason}'`, d, mstimeout )
    }
  } )
  utils.lognewchannel()

  const channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": clienta.local.port, "codec": 0 } }, async function( d ) {
    if( "close" === d.action ) {
      utils.logclosechannel( `Echo with record for ${mstimeout} mS completed with reason '${d.reason}' (power)`, d, mstimeout )
      await fs.promises.unlink( recording ).catch( () => {} )
      await fs.promises.unlink( powerrecording ).catch( () => {} )
    }
  } )
  clienta.remote( { "address": "localhost", "port": channela.local.port, "codec": acodec } )
  utils.lognewchannel()

  clienta.play( { "files": [ { "wav": "/tmp/powerdetectprofile.wav" } ] } )

  setTimeout( () => {
    clienta.close()
  }, mstimeout )

  await utils.waitbetween( 0, 500 )
  channela.echo()

  await utils.waitbetween( 0, 500 )
  channela.record( {
    "file": recording
  } )

  await utils.waitbetween( 0, 500 )
  channela.record( {
    "file": powerrecording,
    "startabovepower": 250,
    "finishbelowpower": 200,
    "minduration": 2000,
    "maxduration": 15000,
    "poweraveragepackets": 20
  } )
}
