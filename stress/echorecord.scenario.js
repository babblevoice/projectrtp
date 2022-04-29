
const projectrtp = require( "../index.js" ).projectrtp
const fs = require( "fs" )
const dgram = require( "dgram" )
const utils = require( "./utils.js" )

module.exports = ( packets = utils.between( 50, 50*60*5 ) ) => {

  utils.log( `Starting echo with record for ${packets} packets` )
  const client = dgram.createSocket( "udp4" )
  client.bind()

  let recording = "/tmp/" + utils.mktemp() + ".wav"

  client.on( "listening", async function() {

    let ourport = client.address().port

    let channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, async function( d ) {
      if( "close" === d.action ) {
        utils.cancelremainingscheduled( client )
        client.close()
        utils.logclosechannel( `Echo with record for ${packets} packets completed with reason '${d.reason}'` )
        await new Promise( ( resolve, reject ) => { fs.unlink( recording, ( err ) => { resolve() } ) } )
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

    /* send a packet every 20mS x 50 */
    for( let i = 0;  i < packets; i ++ ) {
      utils.sendpk( i, i * 20, channel.local.port, client )
    }
  } )
}
