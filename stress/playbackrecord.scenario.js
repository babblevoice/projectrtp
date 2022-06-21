
// TODO - need to send data to keep alive and close properly - not on idle

const projectrtp = require( "../index.js" ).projectrtp
const fs = require( "fs" )
const utils = require( "./utils.js" )

/*
  clienta (echo) ---> channela (play/record)
*/

module.exports = async ( mstimeout ) => {

  const acodec = utils.randcodec()

  utils.log( `Starting playback with record for ${mstimeout} mS` )
  let recording = utils.mktempwav()

  const clienta = await projectrtp.openchannel( {}, ( d ) => {
    if( "close" === d.action ) {
      channela.close()
      utils.logclosechannel( `Mix 2 (clienta) for ${mstimeout} mS completed with reason '${d.reason}'`, d, mstimeout )
    }
  } )
  utils.lognewchannel()

  const channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": clienta.local.port, "codec": acodec } }, async ( d ) => {
    if( "close" === d.action ) {
      utils.logclosechannel( `Playback with record for ${mstimeout} mS completed with reason '${d.reason}'`, d, mstimeout )
      await fs.promises.unlink( recording ).catch( () => {} )
    }
  } )
  clienta.remote( { "address": "localhost", "port": channela.local.port, "codec": acodec } )
  clienta.echo()
  utils.lognewchannel()

  setTimeout( () => {
    clienta.close()
  }, mstimeout )

  await utils.waitbetween( 0, 500 )
  channela.play( {
    "loop": true,
    "files": [
      { "wav": "/tmp/ukringing.wav" }
    ]
  } )

  await utils.waitbetween( 0, 500 )
  channela.record( {
    "file": recording
  } )


}
