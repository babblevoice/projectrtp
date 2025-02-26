

const dgram = require( "dgram" )

const projectrtp = require( "../index.js" ).projectrtp
const fs = require( "fs" )
const utils = require( "./utils.js" )

/*
  js client (too fast) ---> channela (play/record)
*/

function sendpk( sn, sendtime, dstport, server, data = undefined, pt = 0, ssrc = 25 ) {

  const pklength = 172

  return setTimeout( () => {

    let payload
    if( undefined !== data ) {
      payload = data
    } else {
      payload = Buffer.alloc( pklength - 12 ).fill( projectrtp.codecx.linear162pcmu( sn ) & 0xff )
    }

    const subheader = Buffer.alloc( 10 )

    const ts = sn * 160

    subheader.writeUInt16BE( ( sn + 100 ) % ( 2**16 ) /* just some offset */ )
    subheader.writeUInt32BE( ts, 2 )
    subheader.writeUInt32BE( ssrc, 6 )

    const header = Buffer.from( [ 0x80, 0x00 ] )
    header.writeUInt8( pt, 1 ) // payload type

    const rtppacket = Buffer.concat( [
      header,
      subheader,
      payload ] )

    server.send( rtppacket, dstport, "localhost" )
  }, sendtime * 17 )
}

async function createclient() {
  const client = dgram.createSocket( "udp4" )
  client.on( "message", function( msg ) {
  } )

  client.bind()
  await new Promise( ( r ) => { client.on( "listening", function() { r() } ) } )

  return client
}

module.exports = async ( mstimeout ) => {

  const acodec = utils.randcodec()

  utils.log( `Starting playback with record for ${mstimeout} mS but sending packets too quickly` )
  const recording = utils.mktempwav()

  const clienta = await createclient()
  const clientb = await createclient()

  const channela = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": clienta.address().port, "codec": acodec } }, async ( d ) => {
    if( "close" === d.action ) {
      utils.logclosechannel( `Playback with record for ${mstimeout} mS completed with reason '${d.reason}'`, d, mstimeout )
      await fs.promises.unlink( recording ).catch( () => {} )
    }
  } )

  const channelb = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": clientb.address().port, "codec": acodec } }, async ( d ) => {
    if( "close" === d.action ) {
      utils.logclosechannel( `Playback with record for ${mstimeout} mS completed with reason '${d.reason}'`, d, mstimeout )
      await fs.promises.unlink( recording ).catch( () => {} )
    }recording
  } )

  utils.lognewchannel()
  utils.lognewchannel()

  const pkcount = Math.floor( mstimeout / 20 ) 
  for( let i = 0; i < pkcount; i++ ) {
    sendpk( i, i, channela.local.port, clienta, )
  }

  for( let i = 0; i < pkcount; i++ ) {
    sendpk( i, i, channelb.local.port, clientb, )
  }

  await utils.waitbetween( 0, 500 )
  channela.mix( channelb )

  await utils.waitbetween( 0, 500 )
  channela.record( {
    "file": recording,
    "numchannels": utils.between( 0, 1 )
  } )

  await utils.wait( mstimeout )

  clienta.close()
  channela.close()
}
