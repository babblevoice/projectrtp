
const net = require( "net" )
const { v4: uuidv4 } = require( "uuid" )
const message = require( "./message.js" )

const instance = uuidv4()
const channels = new Map()

const channelmap = {
  "close": ( chan ) => chan.close(),
  "remote": ( chan, msg ) => chan.remote( msg.remote ),
  "mix": ( chan, msg ) => {
    if( channels.has( msg.other.uuid ) ) {
      chan.mix( channels.get( msg.other.uuid ) )
    }
  },
  "unmix": ( chan ) => chan.unmix(),
  "dtmf": ( chan, msg ) => chan.dtmf( msg.digits ),
  "echo": ( chan ) => chan.echo(),
  "play": ( chan, msg ) => chan.play( msg.soup ),
  "record": ( chan, msg ) => chan.record( msg.options ),
  "direction": ( chan, msg ) => chan.direction( msg.options )
}

/**
 * One for each connection spawned by rtpnode.
 */
class rtpnodeconnection {
  constructor( parent, connection ) {
    this.connectionid = uuidv4()
    this.connection = connection
    this.connectionlength = 0
    this.mode = "listen"
    this.parent = parent
    this.messagestate = message.newstate()
    this._reconnecttime = 500 /* mS */
    this._destroying = false

    // spawned from listen
    if( connection ) {
      connection.setKeepAlive( true )
      connection.on( "data", this._onsocketdata.bind( this ) )
    }
  }

  static create( parent, connection ) {
    return new rtpnodeconnection( parent, connection )
  }

  destroy() {
    this._destroying = true
    if( this._reconnecttimerid ) {
      clearTimeout( this._reconnecttimerid )
      delete this._reconnecttimerid
    }

    this.connection.destroy()
  }

  _onsocketdata( data ) {
    message.parsemessage( this.messagestate, data, ( msg ) => {
      try {
        this.parent._pre( msg, ( modifiedmsg ) => {
          this._processmessage( modifiedmsg )
        } )
      } catch( e ) {
        console.error( "Unhandled exception in babble-rtp", e )
      }
    } )
  }

  /**
   * @private
   * @param { object } msg
   * @returns boolean
   */
  _updatechannel( msg ) {

    if( undefined === msg.channel ) return false
    if( undefined === msg.uuid ) return false

    const chan = channels.get( msg.uuid )
    if( undefined == chan ) return false

    if( msg.channel in channelmap ) {
      channelmap[ msg.channel ]( chan, msg )
    } else {
      const channelidentifiers = {
        "id": msg.id,
        "uuid": msg.uuid
      }
      this.send( { ...{ "error": "Unknown method" }, ...channelidentifiers } )
    }

    return true
  }

  /**
   * 
   * @param { object } msg 
   * @returns { Promise< Boolean > }
   */
  async _processmessage( msg ) {
    if( "open" == msg.channel ) return await this._openchannel( msg )

    return this._updatechannel( msg )
  }

  /**
   * Send a message back to the main server, include stats to help with load balancing.
   * @param { object } msg
   * @returns { void }
   */
  send( msg, cb = undefined ) {
    this.parent._post( msg, ( modifiedmsg ) => {
      if( this._destroying ) return
      msg.status = this.parent.prtp.stats()
      msg.status.instance = instance
      this.connection.write( message.createmessage( modifiedmsg ) )
      if (cb !== undefined) cb(msg)
    } )
  }


  /**
   * @private
   * @param { object } msg
   * @returns { Promise< Boolean > }
   */
  async _openchannel( msg ) {
    this.connectionlength += 1
    msg.forcelocal = true

    const chan = await this.parent.prtp.openchannel( msg, ( cookie ) => {
      // TODO: we might want to ensure the actual event has been written
      // to the server before cleaning up the channel on our side?
      this.send( { ...{ "id": chan.id, "uuid": chan.uuid }, ...cookie },
        ( cookie ) => {
          if ( "close" === cookie.action )  {
            this.connectionlength -= 1
            channels.delete( chan.uuid )

            if( 0 == this.connectionlength && "listen" == this.mode ) {
              this.parent.connections.delete( this.connectionid )
              this.connection.destroy()
            }
          }
        } )
    } )
    channels.set( chan.uuid, chan )
    this.send( { ...chan, ...{ "action": "open" } } )

    return true
  }


  /**
   * @summary Connect to a server.
   * @param {string} host
   * @param {number} port
   * @return {Promise} - Promise which resolves to an rtpnode
  */
  connect( host, port ) {
    this.host = host
    this.port = port
    this.mode = "connect"

    return new Promise( resolve => {

      this.connection = net.createConnection( this.port, this.host )
      this.connection.on( "data", this._onsocketdata.bind( this ) )

      this.connection.setKeepAlive( true )

      this.connection.on( "connect", () => {
        this.send( {} )
        resolve()
        this._reconnecttime = 500 /* mS */
      } )
      
      this.connection.on( "error", () => {
        if( this._destroying ) return
        this._runreconnect()
      } )
      this.connection.on( "close", () => {
        if( this._destroying ) return
        this._runreconnect()
      } )
    } )
  }

  /**
   * @private
   * @returns { void }
   */
  _reconnect() {
    delete this._reconnecttimerid
    this.connect( this.host, this.port )
  }
  
  /**
   * @private
   * @returns { void }
   */
  _runreconnect() {
    if( this._reconnecttimerid ) return
    console.log( "Disconnected - trying to reconnect to " + this.host + ":" + this.port )

    this._reconnecttimerid = setTimeout( this._reconnect.bind( this ), this._reconnecttime )

    this._reconnecttime = this._reconnecttime * 2
    if( this._reconnecttime > ( 1000 * 2 ) ) this._reconnecttime = this._reconnecttime / 2
  }

}

/**
 * @summary An RTP node. We are a remote RTP node which waits for intruction from our server.
 */
class rtpnode {
  /**
   * 
   * @param { object } prtp 
   * @param { string } address 
   * @param { number } port 
   */
  constructor( prtp, address = "", port = -1 ) {
    this.prtp = prtp
    this.address = address
    this.port = port
    this.instance = uuidv4()
    this.connections = new Map()

    /* pre callbacks are called when we receive an instruction from
    our server and we want to check if there is anything to do such as
    download a file from a remote storage facility or generate a wav file
    from a TTS engine. */
    this._pre = this._defaulthandler.bind( this )

    /* post callbacks are called when our addon wants to pass something back
    to our server - this is to, for example, upload a recording to a remote storage
    facility. */
    this._post = this._defaulthandler.bind( this )
  }

  /**
   * @summary Connect to a server.
   * @param {string} host
   * @param {number} port
   * @return {Promise<rtpnode>} - Promise which resolves to an rtpnode
  */
  async connect( host, port ) {

    const con = rtpnodeconnection.create( this )
    await con.connect( host, port )
    this.connections.set( con.connectionid, con )
    return this
  }

  /**
   * @summary Listen for new connections ( when openchannel is called in lib/server.js ).
   */
  listen() {
    let listenresolve
    const listenpromise = new Promise( ( r ) => listenresolve = r )
    this.server = net.createServer( ( connection ) => {

      const con = rtpnodeconnection.create( this, connection )
      this.connections.set( con.connectionid, con )
    } )

    this.server.listen( this.port, this.address )
    this.server.on( "listening", () => listenresolve() )
    this.server.on( "close", () => {} )
    return listenpromise
  }

  /**
  * This callback is displayed as a global member.
  * @callback requestComplete
  * @param {object} message
  */

  /**
   * This callback is displayed as a global member.
   * @callback requestCallback
   * @param {object} message
   * @param {requestComplete} cb - message passed back into projectrtp framework to process
  */

  /**
   * @summary When a message is sent to our node - pre process any request
   * in the callback. Useful for things like downloading actual wav files
   * or create wav files using a TTS engine etc. This pre and post processing
   * is only available when running the RTP server as a node.
   * @param {requestCallback} cb - The callback that handles the response.
   * @returns { void }
   */
  onpre( cb ) {
    this._pre = cb
  }

  /**
   * @summary When a message is sent back to our server - post process any request
   * in the callback before final transmission to the server. Useful for uploading
   * recordings or other processing.
   * @param {requestCallback} cb - The callback that handles the response.
   * @returns { void }
   */
  onpost( cb ) {
    this._post = cb
  }

  /**
   * @summary Destroy this node
   * @returns { void }
   */
  destroy() {
    this._destroying = true
    nodeinterface.clean()

    this.connections.forEach( ( con ) => con.destroy() )
    this.connections.clear()

    if ( this.server ) this.server.close()
  }

  /**
   * 
   * @param { object } msg 
   * @param { function } cb 
   */
  _defaulthandler( msg, cb ) {
    cb( msg )
  }
}

/**
 * Node Interface. Use this if we are operating as a remote node to a central management server (server.interface running elsewhere).
 * Use this to either listen for inbound connections from a remote server.interface or activly connect back to a remote server.interface which is listening for connections.
 * @alias node.interface
 */
class nodeinterface {

  /**
   * @private
   * @type { rtpnode }
   */
  static _n

  /**
   * @hideconstructor
   * @param { object } prtp 
   */
  constructor( prtp ) {
    this.prtp = prtp
  }

  /**
   * 
   * @param { object } prtp 
   * @returns nodeinterface
   * @ignore
   */
  static create( prtp ) {
    return new nodeinterface( prtp )
  }

  /**
   * Connect to a listening server.
   * @param { number } port 
   * @param { string } host 
   * @returns { Promise< rtpnode > }
   */
  connect( port = 9002, host ) {

    if( nodeinterface._n ) return new Promise( r => r( nodeinterface._n ) )

    nodeinterface._n = new rtpnode( this.prtp )
    return nodeinterface._n.connect( host, port )
  }

  /**
   * listen to allow a server connect to us.
   * @param { string } address 
   * @param { number } port 
   * @returns { Promise< rtpnode > }
   */
  async listen( address, port ) {

    if( nodeinterface._n ) return nodeinterface._n

    nodeinterface._n = new rtpnode( this.prtp, address, port )
    await nodeinterface._n.listen()

    return nodeinterface._n
  }

  /**
   * Returns a node (connect must have been called first). For testing only.
   * @returns { rtpnode }
   * @ignore
   */
  get() {
    return nodeinterface._n
  }

  /**
   * Clean up references
   */
  static clean() {
    delete nodeinterface._n
  }

  /**
   * Close connections including clean up references
   * @returns { void }
   */
  static destroy() {
    if( !nodeinterface._n ) return
    nodeinterface._n.destroy()
  }
}

module.exports.rtpnode = rtpnode
module.exports.interface = nodeinterface

