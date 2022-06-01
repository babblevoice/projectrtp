
// TODO - need to send data to keep alive and close properly - not on idle

const projectrtp = require( "../index.js" ).projectrtp
const fs = require( "fs" )
const dgram = require( "dgram" )
const utils = require( "./utils.js" )

module.exports = ( packets ) => {
  utils.log( `Starting playback with record for ${packets} packets` )
  const client = dgram.createSocket( "udp4" )

  let recording = "/tmp/" + utils.mktemp() + ".wav"

  client.bind()
  client.on( "listening", async function() {

    let ourport = client.address().port

    let channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, async function( d ) {
      if( "close" === d.action ) {
        utils.cancelremainingscheduled( client )
        client.close()
        utils.logclosechannel( `Playback with record for ${packets} packets completed with reason '${d.reason}'` )
        await new Promise( ( resolve, reject ) => { fs.unlink( recording, ( err ) => { resolve() } ) } )
      }
    } )
    utils.lognewchannel()

    let receviedpkcount = 0
    client.on( "message", function( msg, rinfo ) {
      receviedpkcount++
      if( receviedpkcount >= packets ) {
        channel.close()
      }
    } )

    await utils.waitbetween( 0, 500 )
    channel.play( {
      "loop": true,
      "files": [
        { "wav": "/tmp/ukringing.wav" }
      ]
    } )

    await utils.waitbetween( 0, 500 )
    channel.record( {
      "file": recording
    } )

    /* send a packet every 20mS x 50 */
    for( let i = 0;  i < packets; i ++ ) {
      utils.sendpk( i, i * 20, channel.local.port, client )
    }
  } )
}
