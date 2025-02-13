
const projectrtp = require( "../index.js" ).projectrtp
const fs = require( "fs" )
const utils = require( "./utils.js" )

/*
  clienta (play) ---> channela (echo/record)
*/

module.exports = async ( mstimeout ) => {

  utils.log( `Starting echo with record for ${mstimeout} mS` )

  const recording = utils.mktempwav()
  const codeca = utils.randcodec()

  const clienta = await projectrtp.openchannel( {}, ( d ) => {
    if( "close" === d.action ) {
      channela.close()
      utils.logclosechannel( `Mix 2 (clienta) for ${mstimeout} mS completed with reason '${d.reason}'`, d, mstimeout )
    }
  } )
  utils.lognewchannel()

  const channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": clienta.local.port, "codec": codeca } }, async ( d ) => {
    if( "close" === d.action ) {
      utils.logclosechannel( `Echo with record for ${mstimeout} mS completed with reason '${d.reason}'`, d, mstimeout )
      await fs.promises.unlink( recording ).catch( () => {} )
    }
  } )

  clienta.remote( { "address": "localhost", "port": channela.local.port, "codec": codeca } )
  utils.lognewchannel()
  clienta.play( { "files": [ { "loop": true, "wav": "/tmp/powerdetectprofile.wav" } ] } )

  setTimeout( () => {
    clienta.close()
  }, mstimeout )

  await utils.waitbetween( 0, 500 )
  channela.echo()

  await utils.waitbetween( 0, 500 )
  /* include settings which terminate the recording before OR after the channel closes */
  channela.record( {
    "file": recording,
    "maxduration": utils.waitbetween( 50, mstimeout * 2 ),
    "finishbelowpower": 200
  } )

}
