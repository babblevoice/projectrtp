
const net = require( "net" )
const { v4: uuidv4 } = require( "uuid" )
const message = require( "./message.js" )

const instance = uuidv4()

/**
@summary An RTP node. We are a remote RTP node which waits for
intruction from our server.
@memberof proxy
@hideconstructor
*/
class rtpnode {
  constructor( prtp ) {
    this.prtp = prtp
    this.instance = uuidv4()
  }

  /**
  @summary Connect to a server.
  @param {number} port
  @param {string} host
  @return {Promise<rtpnode>} - Promise which resolves to an rtpnode
  */
  connect( port, host ) {
    this.host = host
    this.port = port

    return new Promise( resolve => {
      this._onsocketreadypromiseresolve = resolve
      this.connection = net.createConnection( this.port, this.host )

      this.connection.on( "connect", this._onsocketconnect.bind( this ) )
      this.connection.on( "data", this._onsocketdata.bind( this ) )
    } )
  }

  /**
  @summary Destroy this node
  */
  destroy() {
    this.connection.destroy()
  }

  _onsocketconnect() {
    console.log( "node._onsocketconnect" )
    this._sendmessage( {} )
    this._onsocketreadypromiseresolve( this )
  }

  _sendmessage( msg ) {

    msg.status = this.prtp.stats()
    msg.status.instance = instance

    this.connection.write( message.createmessage( msg ) )
  }

  _onsocketdata( data ) {
    console.log( "_onsocketdata" )
  }
}

module.exports.connect = ( prtp, port = 9002, host ) => {
  let n = new rtpnode( prtp )
  return n.connect( port, host )
}
