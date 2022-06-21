
const projectrtp = require( "../index.js" ).projectrtp
const fs = require( "fs" )
const utils = require( "./utils.js" )
const expect = require( "chai" ).expect

/*
  clienta ---> channela ---> mix ---> channelb ---> clientb (and we echo back here)
*/

module.exports = async ( mstimeout ) => {
  const acodec = utils.randcodec()

  utils.log( `Starting echo with dual record for ${mstimeout} mS (pause stop)` )

  const recording = utils.mktempwav()
  const secondrecording = utils.mktempwav()

  const clienta = await projectrtp.openchannel( {}, ( d ) => {
    if( "close" === d.action ) {
      channela.close()
      utils.logclosechannel( `Mix 2 (clienta) for ${mstimeout} mS completed with reason '${d.reason}'.` +
      ` Expected number of packets: ${Math.round(mstimeout / 20)}, Received: ${d.stats.in["count"]},` +
      ` Score: ${(d.stats.in["count"] / mstimeout * 20).toFixed(2)}` )
    }
  } )
  utils.lognewchannel()

  const channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": clienta.local.port, "codec": acodec } }, async function( d ) {

    if( "close" === d.action ) {
      utils.logclosechannel( `Echo with record for ${mstimeout} mS completed with reason '${d.reason}' (pause stop)'.` +
      ` Expected number of packets: ${Math.round(mstimeout / 20)}, Received: ${d.stats.in["count"]},` +
      ` Score: ${(d.stats.in["count"] / mstimeout * 20).toFixed(2)}` )
      await fs.promises.unlink( recording ).catch( () => {} )
      await fs.promises.unlink( secondrecording ).catch( () => {} )
    }
  } )
  clienta.remote( { "address": "localhost", "port": channela.local.port, "codec": acodec } )
  expect( clienta.play( { "loop": true, "files": [ { "wav": "/tmp/ukringing.wav" } ] } ) ).to.be.true
  utils.lognewchannel()

  setTimeout( () => {
    clienta.close()
  }, mstimeout )

  await utils.waitbetween( 0, 500 )
  channela.echo()

  await utils.waitbetween( 0, 500 )
  channela.record( {
    "file": recording
  } )
  utils.log( "Requested first recording" )

  await utils.waitbetween( 0, 500 )
  channela.record( {
    "file": secondrecording
  } )
  utils.log( "Requested second recording" )

  setTimeout( () => {
    channela.record( {
      "file": recording,
      "pause": true
    } )
  }, utils.between( 0, mstimeout / 2 ) )

  setTimeout( () => {
    channela.record( {
      "file": secondrecording,
      "finish": true
    } )
  }, utils.between( 0, mstimeout / 2 ) )
}
