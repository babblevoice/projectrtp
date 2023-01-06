

/*
A message consists of a header followed by JSON data.
Header (5 byte)
{
  uint_8 magic: 0x33,
  uint_16 version: 0,
  uint_16 size (BE)
}
*/

/**
 * @typedef { object } messagestate
 * @property { number } state
 * @property { Buffer } buffer
 * @property { number } bodylength
 */

/**
 * @returns { messagestate }
 */
module.exports.newstate = () => {
  return {
    "state": 0,
    "buffer": Buffer.alloc( 0 ),
    "bodylength": 0
  }
}

/**
 * 
 * @param { object } state 
 * @returns { void }
 * @ignore
 */
function lookforpacketstart( state ) {
  if( 0 === state.state && 5 <= state.buffer.length ) {
    const dataheader = state.buffer.slice( 0, 5 )

    state.buffer = state.buffer.slice( 5 )
    if ( 0x33 == dataheader[ 0 ] ) {

      state.bodylength = dataheader.readInt16BE( 3 )
      state.state = 1
    } else {
      console.error( "ProjectRTP Bad Magic - this shouldn't happen" )
      state.state = 2
    }
  }
}

/**
 * 
 * @param { object } state 
 * @param { function } cb
 * @returns { void }
 * @ignore
 */
function trimstateandcallback( state, cb ) {
  if ( 1 === state.state && 0 < state.buffer.length ) {
    if ( state.buffer.length >= state.bodylength ) {
      state.state = 0

      const msgbody = state.buffer.slice( 0, state.bodylength ).toString()
      // so that we simpy just don't index the old full buffer which grows and grows
      state.buffer = Buffer.from( state.buffer.slice( state.bodylength ) )

      try {
        const msg = JSON.parse( msgbody )
        if( cb ) cb( msg )
        /* In case we have more than 1 queued */
        return module.exports.parsemessage( state, Buffer.alloc( 0 ), cb )
      } catch( e ) {
        if( e instanceof SyntaxError ) {
          return
        }
        throw e
      }
    }
  }
}

/**
 * @typedef { function } messagecallback
 * @param { object } msg - parsed js object
 * @returns { void }
 */

/**
 * @param { messagestate } state 
 * @param { Buffer } newdata 
 * @param { messagecallback } cb 
 * @returns { void }
 */
module.exports.parsemessage = ( state, newdata, cb ) => {

  if ( 2 == state.state ) {
    state.state = 0
    return
  }

  state.buffer = Buffer.concat( [ state.buffer, newdata ] )

  lookforpacketstart( state )
  if( 2 == state.state ) return
  trimstateandcallback( state, cb )
}

/**
 * 
 * @param { object } obj 
 * @returns { Buffer }
 */
module.exports.createmessage = ( obj ) => {

  const d = Buffer.concat( [ Buffer.from( [ 0x33, 0x00, 0x00, 0x00, 0x00 ] ),
    Buffer.from( JSON.stringify( obj ) ) ] )

  d.writeInt16BE( d.length - 5, 3 )
  return d
}
