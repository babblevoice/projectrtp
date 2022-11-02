
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
@summary An RTP node. We are a remote RTP node which waits for
intruction from our server.
@memberof proxy
@hideconstructor
*/
class rtpnode {
  constructor( prtp ) {
    this.prtp = prtp
    this.instance = uuidv4()
    this.messagestate = message.newstate()

    /* pre callbacks are called when we receive an instruction from
    our server and we want to check if there is anything to do such as
    download a file from a remote storage facility or generate a wav file
    from a TTS engine. */
    this._pre = this._defaulthandler.bind( this )

    /* post callbacks are called when our addon wants to pass something back
    to our server - this is to, for example, upload a recording to a remote storage
    facility. */
    this._post = this._defaulthandler.bind( this )

    this._reconnecttimerid = false
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
    this._destroying = false
    this._reconnecttime = 500 /* mS */

    return new Promise( resolve => {
      this._onsocketreadypromiseresolve = resolve
      this.connection = net.createConnection( this.port, this.host )
      this.connection.setKeepAlive( true )

      this.connection.on( "connect", this._onsocketconnect.bind( this ) )
      this.connection.on( "data", this._onsocketdata.bind( this ) )
      this.connection.on( "error", this._onsocketerror.bind( this ) )
      this.connection.on( "close", this._onsocketclose.bind( this ) )
    } )
  }

    /**
  @summary Listen for new connections ( when openchannel is called in lib/server.js ).
  */
  listen( port=9002, host="::" ) {
    this.server = net.createServer( this._onnewconnection.bind( this ) )

    this.server.listen( port, host )
    this.server.on( "listening", this._onsocketlistening.bind( this ) )
    this.server.on( "close", this._oncloseconnection.bind( this ) )
  }

  _onsocketlistening() {
  }

  _oncloseconnection() {
    if( this._closingpromiseresolve ) this._closingpromiseresolve()
    this._closingpromiseresolve = false
  }

  _onnewconnection( sock ) {

    sock.setKeepAlive( true )

    sock.on( "error", function ( e ) {
      console.error( e )
    } )

    let state = message.newstate()

    sock.on( "data", ( data ) => {

      message.parsemessage( state, data, ( receivedmsg ) => {

        if( this._onnewconnectionpromise ) {
          this._onnewconnectionpromise()
          this._onnewconnectionpromise = false
        }

        if( undefined !== receivedmsg.id ) {
          if( channels.has( receivedmsg.id ) ) {
            let chan = channels.get( receivedmsg.id )
            chan._on( receivedmsg )
          }
        }

        if( this.em ) this.em.emit( "projectrtp.msg", receivedmsg )
      } )
    } )
  }

  /**
  This callback is displayed as a global member.
  @callback requestComplete
  @param {object} message
  */

  /**
  This callback is displayed as a global member.
  @callback requestCallback
  @param {object} message
  @param {requestComplete} cb - message passed back into projectrtp framework to process
  */

  /**
  @summary When a message is sent to our node - pre process any request
  in the callback. Useful for things like downloading actual wav files
  or create wav files using a TTS engine etc. This pre and post processing
  is only available when running the RTP server as a node.
  @param {requestCallback} cb - The callback that handles the response.
  */
  onpre( cb ) {
    this._pre = cb
  }

  /**
  @summary When a message is sent back to our server - post process any request
  in the callback before final transmission to the server. Useful for uploading
  recordings or other processing.
  @param {requestCallback} cb - The callback that handles the response.
  */
  onpost( cb ) {
    this._post = cb
  }

  /**
   * Send a message back to the main server, include stats to help with load balancing.
   * @param { object } msg 
   */
  send( msg ) {
    this._post( msg, ( modifiedmsg ) => {
      if( this._destroying ) return
      msg.status = this.prtp.stats()
      msg.status.instance = instance
      this.connection.write( message.createmessage( modifiedmsg ) )
    } )
  }

  /**
  @summary Destroy this node
  */
  destroy() {
    this._destroying = true

    if( this._reconnecttimerid ) clearTimeout( this._reconnecttimerid )
    this.connection.destroy()
  }

  _onsocketconnect() {
    console.log( "Connected to " + this.host + ":" + this.port )
    this.send( {} )
    this._onsocketreadypromiseresolve( this )
    //console.log( "RTP node connected to control server " + this.host + ":" + this.port )
    this._reconnecttime = 500 /* mS */
  }

  async _processmessage( msg ) {
    return ( await this._openchannel( msg ) || this._updatechannel( msg ) )
  }

  _defaulthandler( msg, cb ) {
    cb( msg )
  }

  _reconnect() {

    this._reconnecttimerid = false
    this.connection = net.createConnection( this.port, this.host )
        .on( "error", () => {
          this._runreconnect()
        } )

    this.connection.setKeepAlive( true )

    this.connection.on( "connect", this._onsocketconnect.bind( this ) )
    this.connection.on( "data", this._onsocketdata.bind( this ) )
    this.connection.on( "error", this._onsocketerror.bind( this ) )
    this.connection.on( "close", this._onsocketclose.bind( this ) )
  }

  _runreconnect() {
    if( this._reconnecttimerid ) return
    console.log( "Disconnected - trying to reconnect to " + this.host + ":" + this.port )

    this._reconnecttimerid = setTimeout( this._reconnect.bind( this ), this._reconnecttime )

    this._reconnecttime = this._reconnecttime * 2
    if( this._reconnecttime > ( 1000 * 2 ) ) this._reconnecttime = this._reconnecttime / 2
  }

  _onsocketclose() {
    if( this._destroying ) return
    this._runreconnect()
  }

  _onsocketerror() {
    if( this._destroying ) return
    this._runreconnect()
  }

  _onsocketdata( data ) {
    message.parsemessage( this.messagestate, data, ( msg ) => {
      try {
        this._pre( msg, ( modifiedmsg ) => {
          this._processmessage( modifiedmsg )
        } )
      } catch( e ) {
        console.error( "Unhandled exception in babble-rtp", e )
      }
    } )
  }

  async _openchannel( msg ) {
    if( "open" !== msg.channel ) return false
    msg.forcelocal = true

    let chan = await this.prtp.openchannel( msg, ( x ) => {
      if( "close" === x.action ) channels.delete( chan.uuid )
      this.send( { ...{ "id": chan.id, "uuid": chan.uuid }, ...x } )
    } )

    channels.set( chan.uuid, chan )
    this.send( { ...chan, ...{ "action": "open" } } )

    return true
  }

  _updatechannel( msg ) {

    if( undefined === msg.channel ) return false
    if( undefined === msg.uuid ) return false

    let chan = channels.get( msg.uuid )
    if( undefined == chan ) return false

    if( msg.channel in channelmap ) {
      channelmap[ msg.channel ]( chan, msg )
    } else {
      let channelidentifiers = {
        "id": msg.id,
        "uuid": msg.uuid
      }
      this.send( { ...{ "error": "Unknown method" }, ...channelidentifiers } )
    }

    return true
  }
}

/**
 * Connect to a server.
 */
let n
module.exports.connect = ( prtp, port = 9002, host ) => {
  n = new rtpnode( prtp )
  return n.connect( port, host )
}

/**
 * Returns a node (connect must have been called first).
 * @returns { object }
 */
module.exports.get = () => {
  return n
}

module.exports.listen = async ( port, address, em ) => {

  if( !n ) {
    n = new rtpnode( port )
    n.listen()
  }
  return n
}
