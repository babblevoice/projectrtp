
const projectrtp = require( "../index.js" ).projectrtp

module.exports.log = function( message ) {
  let d = new Date()
  var datestring = (
    "0" + d.getDate()).slice(-2) + "-" + ("0"+(d.getMonth()+1)).slice(-2) + "-" +
    d.getFullYear() + " " + ("0" + d.getHours()).slice(-2) + ":" + ( "0" + d.getMinutes() ).slice( -2 ) + ":" + ( "0" + d.getSeconds() ).slice( -2 )

  console.log( "[" + datestring + "] " + message )
}

let channelcount = 0
let totalcount = 0
module.exports.lognewchannel = () => {
  channelcount++
  module.exports.log( `New channel opened - current count now ${channelcount}` )
}

module.exports.logclosechannel = ( message ) => {
  channelcount--
  totalcount++
  module.exports.log( message )
  module.exports.log( `Channel closed - current count now ${channelcount} total channels this session ${totalcount}` )
}

module.exports.totalchannelcount = () => {
  return totalcount
}

module.exports.currentchannelcount = () => {
  return channelcount
}

module.exports.mktemp = () => {
  return "project_" + (Math.random() + 1).toString(36).substring(7)
}

module.exports.between = ( min, max ) => {
  return Math.floor(
    Math.random() * ( max - min ) + min
  )
}

module.exports.waitbetween = async ( min, max ) => {
  return new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), module.exports.between( min, max ) ) } )
}

module.exports.cancelremainingscheduled = ( dgramsocket ) => {
  if( undefined !== dgramsocket.outscheduled ) {
    dgramsocket.outscheduled.every( ( id ) => {
      clearTimeout( id )
      return true
    } )
  }
}


module.exports.sendpk = ( sn, sendtime, dstport, dgramsocket, ssrc = 25, pklength = 172 ) => {

  if( undefined == dgramsocket.outscheduled ) {
    dgramsocket.outscheduled = []
  }

  dgramsocket.outscheduled.push(
    setTimeout( () => {
      let payload
      if( "number" === typeof pklength ) {
        payload = Buffer.alloc( pklength - 12 ).fill( sn & 0xff )
      } else {
        /* allow the caller to pass in a Buffer */
        payload = pklength
      }

      let ts = sn * 160
      let tsparts = []
      /* portability? */
      tsparts[ 3 ] = ( ts & 0xff000000 ) >> 24
      tsparts[ 2 ] = ( ts & 0xff0000 ) >> 16
      tsparts[ 1 ] = ( ts & 0xff00 ) >> 8
      tsparts[ 0 ] = ( ts & 0xff )

      let snparts = []
      sn = ( sn + 100 ) % ( 2**16 ) /* just some offset */
      snparts[ 0 ] = sn & 0xff
      snparts[ 1 ] = sn >> 8

      let ssrcparts = []
      ssrcparts[ 3 ] = ( ssrc & 0xff000000 ) >> 24
      ssrcparts[ 2 ] = ( ssrc & 0xff0000 ) >> 16
      ssrcparts[ 1 ] = ( ssrc & 0xff00 ) >> 8
      ssrcparts[ 0 ] = ( ssrc & 0xff )


      let rtppacket = Buffer.concat( [
        Buffer.from( [
          0x80, 0x00,
          snparts[ 1 ], snparts[ 0 ],
          tsparts[ 3 ], tsparts[ 2 ], tsparts[ 1 ], tsparts[ 0 ],
          ssrcparts[ 3 ], ssrcparts[ 2 ], ssrcparts[ 1 ], ssrcparts[ 0 ]
         ] ),
        payload ] )

      dgramsocket.send( rtppacket, dstport, "localhost" )
    }, sendtime )
  )
}

module.exports.genpcmutone = ( durationseconds = 0.25, tonehz = 100, samplerate = 16000, amp = 15000 ) => {

  const tonebuffer = Buffer.alloc( samplerate*durationseconds, projectrtp.codecx.linear162pcmu( 0 ) )

  for( let i = 0; i < tonebuffer.length; i++ ) {
    let val = Math.sin( ( i / samplerate ) * Math.PI * tonehz ) * amp
    tonebuffer[ i ] = projectrtp.codecx.linear162pcmu( val )
  }

  return tonebuffer
}
