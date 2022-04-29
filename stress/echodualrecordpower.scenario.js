
const projectrtp = require( "../index.js" ).projectrtp
const fs = require( "fs" )
const dgram = require( "dgram" )
const utils = require( "./utils.js" )


/* generate our data */
const startsilenceseconds = 2
const tonedurationseconds = 2
const endsilnceseconds = 3
const totalseconds = startsilenceseconds + tonedurationseconds + endsilnceseconds
const amplitude = 20000
const lowamplitude = 500
const frequescyhz = 50
const samplingrate = 8000
const soundblocks = Math.floor( ( totalseconds * samplingrate ) / 160 )

const sendbuffer = Buffer.concat( [
  Buffer.alloc( samplingrate*startsilenceseconds, projectrtp.codecx.linear162pcmu( 0 ) ),
  utils.genpcmutone( tonedurationseconds, frequescyhz, samplingrate, amplitude ),
  utils.genpcmutone( endsilnceseconds, frequescyhz, samplingrate, lowamplitude )
] )

module.exports = ( packets = utils.between( 50, 50*60*5 ) ) => {

  utils.log( `Starting echo with dual record for ${packets} packets (power)` )
  const client = dgram.createSocket( "udp4" )
  client.bind()

  let recording = "/tmp/" + utils.mktemp() + ".wav"
  let powerrecording = "/tmp/" + utils.mktemp() + ".wav"

  client.on( "listening", async function() {

    let ourport = client.address().port

    let channel = await projectrtp.openchannel( { "remote": { "address": "localhost", "port": ourport, "codec": 0 } }, async function( d ) {

      if( "close" === d.action ) {
        utils.cancelremainingscheduled( client )
        client.close()
        utils.logclosechannel( `Echo with record for ${packets} packets completed with reason '${d.reason}' (power)` )
        await new Promise( ( resolve, reject ) => { fs.unlink( recording, ( err ) => { resolve() } ) } )
        await new Promise( ( resolve, reject ) => { fs.unlink( powerrecording, ( err ) => { resolve() } ) } )
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
      "file": powerrecording,
      "startabovepower": 250,
      "finishbelowpower": 200,
      "minduration": 2000,
      "maxduration": 15000,
      "poweraveragepackets": 20
    } )

    /* send a packet every 20mS x 50 */
    for( let i = 0;  i < packets; i ++ ) {
      let start = ( i % soundblocks ) * 160
      end = start + 160
      utils.sendpk( i, i * 20, channel.local.port, client, 32, sendbuffer.subarray( start, end ) )
    }
  } )
}
