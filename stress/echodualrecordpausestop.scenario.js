
const projectrtp = require( "../index.js" ).projectrtp
const fs = require( "fs" )
const dgram = require( "dgram" )
const utils = require( "./utils.js" )

module.exports = ( packets ) => {

  utils.log( `Starting echo with dual record for ${packets} packets (pause stop)` )
  const client = dgram.createSocket( "udp4" )
  client.bind()

  let recording = "/tmp/" + utils.mktemp() + ".wav"
  let secondrecording = "/tmp/" + utils.mktemp() + ".wav"

  client.on( "listening", async function() {

    let ourport = client.address().port

    let channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, async function( d ) {

      if( "close" === d.action ) {
        utils.cancelremainingscheduled( client )
        client.close()
        utils.logclosechannel( `Echo with record for ${packets} packets completed with reason '${d.reason}' (pause stop)` )
        await new Promise( ( resolve, reject ) => { fs.unlink( recording, ( err ) => { resolve() } ) } )
        await new Promise( ( resolve, reject ) => { fs.unlink( secondrecording, ( err ) => { resolve() } ) } )
      }
    } )

    let receviedpkcount = 0
    client.on( "message", function( msg, rinfo ) {
      receviedpkcount++
      if( receviedpkcount >= packets ) {
        channel.close()
      }
    } )

    utils.lognewchannel()

    await utils.waitbetween( 0, 500 )
    channel.echo()

    await utils.waitbetween( 0, 500 )
    channel.record( {
      "file": recording
    } )

    await utils.waitbetween( 0, 500 )
    channel.record( {
      "file": secondrecording
    } )

    setTimeout( () => {
      channel.record( {
        "file": recording,
        "pause": true
      } )
    }, utils.between( 0, ( packets * 20 ) / 2 ) )

    setTimeout( () => {
      channel.record( {
        "file": secondrecording,
        "finish": true
      } )
    }, utils.between( 0, ( packets * 20 ) / 2 ) )

    /* send a packet every 20mS x 50 */
    for( let i = 0;  i < packets; i ++ ) {
      utils.sendpk( i, i * 20, channel.local.port, client )
    }
  } )
}
