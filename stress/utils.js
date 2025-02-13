

module.exports.log = function( message ) {
  const d = new Date()
  const datestring = (
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

module.exports.logclosechannel = ( message, d, mstimeout ) => {
  channelcount--
  totalcount++
  module.exports.log( message )

  const score = ( d.stats.in["count"] / mstimeout * 20 )
  let scoremsg = ` Score: ${ score.toFixed( 2 ) }`

  // Colour based on score: red, yellow, green
  if( 0.25 >= score ) scoremsg = "\x1B[31m" + scoremsg
  else if( 0.7 >= score ) scoremsg = "\x1B[33m" + scoremsg
  else scoremsg = "\x1B[32m" + scoremsg
  scoremsg += "\x1B[37m"

  module.exports.log( `Expected number of packets: ${ Math.round( mstimeout / 20 ) }, Received: ${ d.stats.in[ "count" ] },` + scoremsg )
  module.exports.log( `Channel closed - current count now ${ channelcount } total channels this session ${ totalcount }` )
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

module.exports.mktempwav = () => {
  return "/tmp/project_" + (Math.random() + 1).toString(36).substring(7) + ".wav"
}

/**
 * 
 * @param { number } min - lower value 
 * @param { number } max - upper value
 * @returns { number }
 */
module.exports.between = ( min, max ) => {
  return Math.floor(
    Math.random() * ( max - min ) + min
  )
}

/**
 * 
 * @param { number } min - min time mS
 * @param { number } max - max time mS
 * @returns { Promise }
 */
module.exports.waitbetween = async ( min, max ) => {
  await new Promise( ( resolve ) => { setTimeout( () => resolve(), module.exports.between( min, max ) ) } )
}

const possiblecodecs = [ 0, 8, 9, 97 ]
/**
 * Returns random supported CODEC
 * @returns { number }
 */
module.exports.randcodec = () => {
  return possiblecodecs[ module.exports.between( 0, possiblecodecs.length ) ]
}
