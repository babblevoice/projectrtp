const projectrtp = require( "../index.js" ).projectrtp
const utils = require( "./utils.js" )
const expect = require( "chai" ).expect

/*
  clienta (play something) ---> channela (echo back)
*/

module.exports = async ( mstimeout ) => {

  utils.log( `Create 2 channels and send/echo for ${mstimeout} mS with dtls` )

  const acodec = utils.randcodec()

  const clienta = await projectrtp.openchannel( {}, ( d ) => {
    if( "close" === d.action ) {
      channela.close()
      utils.logclosechannel( `DTLS (clienta) for ${mstimeout} mS completed with reason '${d.reason}'`, d, mstimeout )
    }
  } )
  utils.lognewchannel()

  const targeta = {
    "address": "localhost",
    "port": clienta.local.port,
    "codec": acodec,
    "dtls": {
      "fingerprint": {
        "hash": projectrtp.dtls.fingerprint
      },
      "mode": "active"
    }
  }

  const channela = await projectrtp.openchannel( { "remote": targeta }, ( d ) => {
    if( "close" === d.action ) {
      utils.logclosechannel( `DTLS (channela) for ${mstimeout} mS completed with reason '${d.reason}'`, d, mstimeout )
    }
  } )

  const targeta2 = {
    "address": "localhost",
    "port": channela.local.port,
    "codec": acodec,
    "dtls": {
      "fingerprint": {
        "hash": projectrtp.dtls.fingerprint
      },
      "mode": "passive"
    }
  }

  clienta.remote( targeta2 )
  utils.lognewchannel()

  expect( clienta.play( { "loop": true, "files": [ { "wav": "/tmp/ukringing.wav" } ] } ) ).to.be.true
  expect( channela.echo() ).to.be.true

  setTimeout( () => {
    clienta.close()
  }, mstimeout )

}
