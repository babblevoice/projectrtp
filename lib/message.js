

/*
A message consists of a header followed by JSON data.
Header (5 byte)
{
  uint_8 magic: 0x33,
  uint_16 version: 0,
  uint_16 size (BE)
}
*/
module.exports.newstate = () => {
  return {
    "state": 0,
    "buffer": new Buffer.alloc( 0 ),
    "bodylength": 0
  }
}

module.exports.parsemessage = ( state, newdata, cb ) => {

  if ( 2 == state.state ) {
    state.state = 0
    return
  }

  state.buffer = Buffer.concat( [ state.buffer, newdata ] )

  if( 0 === state.state && state.buffer.length >= 5 ) {
    let dataheader = state.buffer.slice( 0, 5 )

    state.buffer = state.buffer.slice( 5 )

    if ( 0x33 == dataheader[ 0 ] ) {

      state.bodylength = dataheader.readInt16BE( 3 )
      state.state = 1
    } else {
      console.error( "ProjectRTP Bad Magic - this shouldn't happen" )
      state.state = 2
      return
    }
  }

  if ( 1 === state.state && state.buffer.length > 0 ) {
    if ( state.buffer.length < state.bodylength ) {
      /* Need to wait for more data */
      return
    }
    else {
      state.state = 0

      let msgbody = state.buffer.slice( 0, state.bodylength ).toString()
      // so that we simpy just don't index the old full buffer which grows and grows
      state.buffer = new Buffer.from( state.buffer.slice( state.bodylength ) )

      try {
        let msg = JSON.parse( msgbody )
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

  return state
}

module.exports.createmessage = ( obj ) => {

  let d = Buffer.concat( [ Buffer.from( [ 0x33, 0x00, 0x00, 0x00, 0x00 ] ),
                Buffer.from( JSON.stringify( obj ) ) ] )

  d.writeInt16BE( d.length - 5, 3 )
  return d
}