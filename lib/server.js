

const net = require( "net" )
const { v4: uuidv4 } = require( "uuid" )

const EventEmitter = require( "node:events" )

const message = require( "./message.js" )

let nodes = new Map()
let channels = new Map() /* track where our channels are */

/*
This class definition serves both to document the channel
definition in our addon and also to to reflect te remote
channel in a remote node.
*/

/**
@summary An RTP session
@memberof projectrtp
@hideconstructor
*/
class channel {

  constructor( request, remotenode, cb, openresolve, openreject ) {
    /**
    @member {string}
    @summary Our unique id identifying this channel. Self generated in the proxy server.
    */
    this.id = uuidv4()
    channels.set( this.id, this )
    this.remotenode = remotenode

    this.history = []

    /**
     * @member { object }
     * @summary other method for caller to use to receive events from our channel
     */
    this.em = new EventEmitter()
    if( cb ) this.em.on( "all", cb )

    this.openresolve = openresolve
    this.openreject = openreject

    this._sockerr = false
    this._write( request )
  }

  /**
  @summary Sets or changes the remote of the RTP stream. This can also
  be passed into the channel.create() object.
  @param {Object} remote - see channel.create
  @return bool
  */
  remote( remote ){
    this._write( {
      "channel": "remote",
      "remote": remote
    } )
  }

  /**
  @summary Close the channel
  */
  close() {
    if ( this.refcount !== undefined )
    {
      if ( this.refcount.channels > 1 )
      {
        this.refcount.channels -= 1
        return
      }
    }
    this._write( {
      "channel": "close",
      "uuid": this.uuid
    } )

    if( undefined !== this.openresolve ) {
      this.openresolve()
      delete this.openresolve
      delete this.openreject
    }
  }

  /**
  @summary Adds another channel to mix with this one
  @param {channel} other
  @returns {boolean}
  */
  mix( other ) {
    this._write( {
      "channel": "mix",
      "other": {
        "id": other.id,
        "uuid": other.uuid
      }
    } )
  }

  /**
  @summary Removes the other channel from an existing mix
  @param {channel} other
  @returns {boolean}
  */
  unmix(){
    this._write( {
      "channel": "unmix",
    } )
  }

  /**
  @summary Send RFC 2833 DTMF digits i.e. channel.dtmf( "#123*" )
  @param {string} digits
  @returns {boolean}
  */
  dtmf( digits ) {
    this._write( {
      "channel": "dtmf",
      "digits": digits
    } )
  }

  /**
  @summary Echos receved RTP back out when unmixed
  @returns {boolean}
  */
  echo() {
    this._write( {
      "channel": "echo"
    } )
  }

  /**
   * @summary individual soundsoup file
   * @typedef { Object } soundsoupfile
   * @property { ( number | boolean ) } [ soundsoupfile.loop ] - continue looping this soup file until other instruction (or the number of loops)
   * @property { String } soundsoupfile.wav - the filename of the soundsoup file
   * @property { number } [ soundsoupfile.start ] - mS into the file where to start playing
   * @property { number } [ soundsoupfile.stop ] - mS into the file where to stop playing
   */

  /**
   * @typedef { Object } soundsoup
   * @property { ( number | boolean ) } [ soundsoup.loop ] continue looping this soup until other instruction (or the number of loops)
   * @param { boolean } [ soundsoup.interupt ] does a telephone-event interupt (end) the playback
   * @param { Array.< soundsoupfile > } soundsoup.files
   */

  /**
   * @summary Plays audio to the channel when unmixed
   * @param { soundsoup } 
   */
  play( soundsoup ){
    this._write( {
      "channel": "play",
      "soup": soundsoup
    } )
  }

  /**
  @summary Plays audio to the channel when unmixed
  @param {Object} options
  @param {string} options.file - filename of the recording
  @param {number} [options.startabovepower] - only start the recording if the average power goes above this value
  @param {number} [options.finishbelowpower] - finish the recording if the average power drops below this level
  @param {number} [options.minduration] - ensure we have this many mS recording
  @param {number} [options.maxduration] - regardless of power options finish when this mS long
  @param {number} [options.poweraveragepackets] - number of packets to average the power calcs over
  @param {boolean} [options.pause] - pause the recording this function can be called again to pause and resume the recording
  @param {boolean} [options.finish=false] - finish the recording
  @returns {boolean}
  */
  record( options ) {
    this._write( {
      "channel": "record",
      "options": options
    } )
  }

  /**
  @summary Enable/disable the sending and receiving of RTP traffic
  @param {Object} options
  @param {boolean} [options.send]
  @param {boolean} [options.recv]
  @returns {boolean}
  */
  direction( options ) {
    this._write( {
      "channel": "direction",
      options
    } )
  }

  // Pseudo interface if a channel is used as a local object
  async openchannel( options, callback ) {
    
    if ( this.refcount === undefined )
    {
      this.refcount = { "channels": 2 }
    }
    else
    {
      this.refcount.channels += 1
    }
    options.refcount = this.refcount
    await s.openchannel( options, callback )
  }

  _on( msg ) {
    msg.timestamp = ( new Date ).getTime()
    this.history.push( msg )

    if( "open" === msg.action ) {
      if( undefined !== this.openresolve ) {
        /**
        @member {Object.remote}
        @summary Stores the remote information - port and address -
        i.e. send this to the peer so they know where to send RTP to..
        @property {object} remote
        @property {string} remote.address - the host address
        @property {number} remote.port
        @property {object} remote.ice
        @property {string} remote.ice.pwd
        @property {object} remote.dtls
        @property {object} remote.dtls.fingerprint
        @property {string} remote.dtls.fingerprint.hash - "aa:ab:ac..."
        @property {string} remote.dtls.fingerprint.type - currently not checked but currnetly MUST be sha-256 
        @property {string} remote.dtls.mode - "active" || "passive"
        */
        this.local = msg.local
        this.action = msg.action

        /**
        @member {string}
        @summary Unique id identifying this channel.
        */
        this.uuid = msg.uuid

        this.openresolve( this )
        delete this.openresolve
        delete this.openreject
      }
      return
    } else if ( "close" === msg.action ) {
      channels.delete( this.id )
    } 

    this.em.emit( "all", msg )
    if( msg.action ) this.em.emit( msg.action, msg )
  }

  _write( msg ) {
    msg.id = this.id
    let uuid = this.uuid
    if( "" !== uuid ) {
      msg.uuid = uuid
    }

    this.remotenode.sock.write( message.createmessage( msg ) )
    
    msg.timestamp = ( new Date ).getTime()
    this.history.push( msg )

    if( this._sockerr ) {
      nodes.delete( this.remotenode.instance )
      try { this.remotenode.sock.close() } catch ( e ) {}
      return false
    }

    return true
  }
}

/* Util */
function between( min, max ) {
  return Math.floor(
    Math.random() * ( max - min ) + min
  )
}

/*
TODO
Make it pick based more on suitable data such as workercount of node or number of used channels etc.
*/
function randomenode() {
  let index = between( 0, nodes.size )
  for( let node of nodes.keys() ) {
    if( 0 === index ) {
      return nodes.get( node )
    }
    index--
  }
}

/**
@summary An RTP server. We are an RTP server which waits for
nodes to connect to us to offer processing power which we can dish out.
@memberof proxy
@hideconstructor
*/
class rtpserver {
  constructor( port, address, em ) {
    this.port = port
    this.address = address
    this.em = em

    this._onnewconnectionpromise = false
    this._closingpromiseresolve = false
  }

  /**
  @summary Listen for new connections.
  */
  listen() {
    this.server = net.createServer( this._onnewconnection.bind( this ) )

    this.server.listen( this.port, this.address )
    this.server.on( "listening", this._onsocketlistening.bind( this ) )
    this.server.on( "close", this._oncloseconnection.bind( this ) )
  }

  /*
  Hidden function which is called by our main project RTP when we have servers
  connected.
  */
  openchannel( options, callback ) {
    return new Promise( ( resolve, reject ) => {
      if( 0 === nodes.size ) {
        console.error( "No available RTP nodes" )
        reject( "No available RTP nodes" )
      }

      let request = JSON.parse( JSON.stringify( options ) )
      request.channel = "open"
      delete request.related
      let related = options.related
      let node
      /* Choose our remote node */
      if ( undefined == related ) {
        node = randomenode()
      } else {
        for( let r in related ) {
          node = channels.get( r.id )
          if( undefined !== node ) {
            break
          }
        }

        if( undefined == node ) {
          node = randomenode()
        }
      }

      let r = new channel( request, node, callback, resolve, reject )
      channels.set( r.id, r )
      if ( options.refcount !== undefined )
      {
        r.refcount = options.refcount
      }

      return
    } )
  }

  /**
  @summary Close and clean up
  */
  destroy() {

    s = false

    return new Promise( resolve => {
      this._closingpromiseresolve = resolve
      for ( const [ key, node ] of nodes.entries() ) {
        node.sock.destroy()
      }

      this.server.close( () => {
        this.server.unref()
        if( this._closingpromiseresolve ) this._closingpromiseresolve()
        this._closingpromiseresolve = false
      } )

      if( this._onnewconnectionpromise ) {
        this._onnewconnectionpromise()
        this._onnewconnectionpromise = false
      }
    } )
  }

  /**
  @summary Return a promise which reslolves on the next new connection.
  Used for testing purposes only - not expected to be used in production.
  @return {Promise}
  */
  waitfornewconnection() {
    return new Promise( resolve => {
      if( nodes.size > 0 ) {
        resolve()
        return
      }
      this._onnewconnectionpromise = resolve
    } )
  }

  _oncloseconnection() {
    if( this._closingpromiseresolve ) this._closingpromiseresolve()
    this._closingpromiseresolve = false
  }

  _onsocketlistening() {
  }

  _onnewconnection( sock ) {

    sock.ournode = {
      "sock": sock,
      "instance": uuidv4(), /* default - which will be overriden */
      "status": {} /* default - which will be overriden */
    }
    sock.setKeepAlive( true )
    nodes.set( sock.ournode.instance, sock.ournode )

    sock.on( "close", () => {
      nodes.delete( ""+sock.ournode.instance )
    } )

    sock.on( "error", function ( e ) {
      console.error( e )
      nodes.delete( sock.ournode.instance )
    } )

    let state = message.newstate()

    sock.on( "data", ( data ) => {

      message.parsemessage( state, data, ( receivedmsg ) => {
        /* Have we been told our node instance id */
        if( undefined !== receivedmsg.instance ) {
          if( !nodes.has( receivedmsg.instance ) ) {
            if( nodes.has( sock.ournode.instance ) ) {
              nodes.delete( ""+sock.ournode.instance )
            }

            sock.ournode.instance = ""+receivedmsg.instance
            nodes.set( sock.ournode.instance, sock.ournode )
            if( undefined !== data.status ) {
              sock.ournode.status = data.status
            }

            if( this._onnewconnectionpromise ) {
              this._onnewconnectionpromise()
              this._onnewconnectionpromise = false
            }
          }
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
}

/**
@summary Get statistics
@return {Object} stats
*/
module.exports.stats = () => {
    return {
      "nodecount": nodes.size
    }
}

module.exports.nodes = () => {
  let nodesdesc = []

  for ( const [ key, node ] of nodes.entries() ) {
    nodesdesc.push( {
      "instance": node.instance,
      "status": node.status
    } )
  }

  return nodesdesc
}

let s = false
module.exports.get = () => {
  return s
}

module.exports.listen = ( port, address, em ) => {

  if( !s ) {
    s = new rtpserver( port, address, em )
    s.listen()
  }
  return s
}
