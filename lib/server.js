

const net = require( "net" )
const { v4: uuidv4 } = require( "uuid" )

const EventEmitter = require( "node:events" )

const message = require( "./message.js" )

let nodes = new Map()
let listeningnodes = []
let channels = new Map() /* track where our channels are */

/*
This class definition serves both to document the channel
definition in our addon and also to to reflect te remote
channel in a remote node.
*/
/**
@summary An RTP session
@memberof projectrtp
@param {string} local.address - Local adress of the channel
@param {number} local.port - Local port of the channel
@hideconstructor
*/
class channel {

  constructor( remotenode ) {
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

  }

  /**
   * Creates a new channel based on a request to open a channel on a remote
   * node which is already connected to us via the listen mechanism.
   */
  static _createforlisten( request, remotenode, cb, openresolve, openreject ) {

    let newchannel = new channel()
    newchannel.remotenode = remotenode
    if( cb ) newchannel.em.on( "all", cb )

    newchannel.openresolve = openresolve
    newchannel.openreject = openreject

    newchannel._sockerr = false
    newchannel._write( request )

    return newchannel
  }

  /**
   * Creates a new channel which connects to a remote rtp node
   * @return { Promise< channel > } - resolves when the connection is connected
   */
  static _createforconnect( request, callback, openresolve, openreject /* do something with me */ ) {

    let newchannel = new channel()
    if( callback ) newchannel.em.on( "all", callback )
    newchannel.connectednode = listeningnodes[ between( 0, listeningnodes.length ) ]
    return new Promise( connectresolve => {

      newchannel.connection = {}
      newchannel.channels = [ newchannel ]
      newchannel.connection.sock = net.createConnection( newchannel.connectednode.port, newchannel.connectednode.host )
      newchannel.connection.sock.setKeepAlive( true )

      newchannel.connection.sock.on( "connect", () => {
        newchannel._write( request )
        /* TODO - add timeout with error */
        if ( connectresolve ) connectresolve( newchannel )
        connectresolve = false
      } )

      let state = message.newstate()
      newchannel.connection.sock.on( "data", ( data ) => {
        message.parsemessage( state, data, ( receivedmsg ) => {
          for ( let chnl of newchannel.channels ) {
            if ( receivedmsg.id === chnl.id ) chnl._on( receivedmsg )
          }
          /* find channel */
          if( openresolve ) { /* if action == open */
            Object.assign( newchannel, receivedmsg )
            openresolve( newchannel )
            openresolve = false
          }
        } )
      } )

      newchannel.connection.sock.on( "error", ( e ) => {
        console.log( e )
      } )

      newchannel.connection.sock.on( "close", () => {
      } )
    } )

  }

  /**
   * This method forces an open channel on the same remote node.
   */
  async openchannel( options, callback ) {

    let request = { "channel": "open" }
    if ( this.connectednode ) {
      let newchannel = new channel()
      newchannel.connection = this.connection
      newchannel.channels = this.channels
      newchannel.channels.push( newchannel )
      newchannel._write( request )
      return newchannel
    } else {
      return await _createforconnect( request, this.remotenode, callback )
    }
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
  async close() {
    /* any outstanding to remove where unmix was not called first? */
    await this._removebridge()

    /* close us */
    this._write( {
      "channel": "close",
      "uuid": this.uuid
    } )

    if ( this.channels ) { 
      // adjust with filter
      const index = this.channels.indexOf( this );
      if (index > -1) this.channels.splice(index, 1)
      if ( this.channels.length === 0 && this.connection.sock) this.connection.sock.destroy()
    }

    if( undefined !== this.openresolve ) {
      this.openresolve()
      delete this.openresolve
      delete this.openreject
    }
    if( this.connection.sock && !this.channels ) {
      this.connection.sock.destroy()
      this.connection.sock = false
    }
  }

  /**
  @summary Adds another channel to mix with this one
  @param {channel} other
  @returns {boolean}
  */
  async mix( other ) {

    if( await this._addbridge( other ) ) return true

    this._write( {
      "channel": "mix",
      "other": {
        "id": other.id,
        "uuid": other.uuid
      }
    } )

    return true
  }

  /**
   * Any channels which have been mixed we create an
   * object common to both channels so we can keep track of channels which 
   * require tidyin up. We try to maintain a star shaped network.
   * In the example diagram, the out mix method mixes between phone1 and this
   * and phone2 and other. 
   * Markdown (mermaid)
      graph TD
        this -->|rtp - our created bridge to link the 2 remote channels| other
        other --> this

        phone1 --> this
        this --> phone1

        phone2 --> other
        other --> phone2
   * @private
   * @param { object } other - other channel
   * @returns 
   */
  async _addbridge( other ) {

    let leftid = this.remotenode.status.instance
    const rightid = other.remotenode.status.instance

    if( !this._bridge && !other._bridge ) {
      this._bridge = other._bridge = {}
      this._bridge.bridges = []
      this._bridge.main = leftid
    }

    if( !this._bridge && other._bridge ) this._bridge = other._bridge
    if( this._bridge && !other._bridge ) other._bridge = this._bridge

    if( this._bridge !== other._bridge ) {
      console.trace( "some complex mixing occured and this function needs finishing" )
      return false
    }

    /* nothing more to do if we are on the same node */
    if( rightid === leftid ) return false

    /* It might have changed now we have linked them up */
    leftid = this._bridge.main

    let found = false
    for( const bridge in this._bridge.bridges ) {
      if( ( bridge.left === leftid || bridge.right === leftid ) &&
          ( bridge.left === rightid || bridge.right === rightid ) ) {
        found = true
        bridge.channels.add( this )
        bridge.channels.add( other )

        this.mix( bridge.left )
        other.mix( bridge.right )
      }
    }

    if( found ) return

    let bridge = {
      /* TODO: this will need reworking with to support reverse cnnection types */
      "left": await s.openchannel( { "nodeinstance": leftid } ),
      "right": await s.openchannel( { "nodeinstance": rightid } ),
      "channels": new Set( [ this, other ] )
    }

    const addressleft = bridge.left.local.privateaddress || bridge.left.local.address
    const addressright = bridge.right.local.privateaddress || bridge.right.local.address
    const g722 = 9

    bridge.left.remote( { "address": addressright, "port": bridge.right.local.port, "codec": g722 } )
    bridge.right.remote( { "address": addressleft, "port": bridge.left.local.port, "codec": g722 } )

    this._bridge.bridges.push( bridge )

    this._write( {
      "channel": "mix",
      "other": {
        "id": bridge.left.id,
        "uuid": bridge.left.uuid
      }
    } )

    other._write( {
      "channel": "mix",
      "other": {
        "id": bridge.right.id,
        "uuid": bridge.right.uuid
      }
    } )

    return true
  }

  /**
   * We check if we are a remote node (i.e.. not the centre of the star)
   * then we close down the link between the two.
   * @private
   * @returns 
   */
  async _removebridge() {

    /* no remote bridges have been created */
    if( !this._bridge || !this._bridge.main ) return

    const usid = this.remotenode.status.instance

    /* The centre of star cannot remove from the network until last man standing */
    if( usid == this._bridge.main ) return

    this._bridge.bridges = await this._bridge.bridges.filter( async ( bridge ) => {
      bridge.channels.delete( this )

      if( bridge.channels.size < 2 ) {

        bridge.left._write( { "channel": "unmix" } )
        bridge.left._write( { "channel": "unmix" } )

        await bridge.left.close()
        await bridge.right.close()
        return false /* remove */
      }
      return true /* keep */
    } )
  }

  /**
  @summary Removes us from an existing mix
  @param {channel} other
  @returns {boolean}
  */
  unmix() {
    this._removebridge()
    this._write( {
      "channel": "unmix",
    } )
    
    return true
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

  /**
   * Called by our socket to pass data back up the chain.
   * @private
   * @param { Buffer } msg 
   */
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

  /**
   * Pass a message to the node.
   * @private
   * @param { object } msg 
   * @returns 
   */
  _write( msg ) {
    msg.id = this.id
    let uuid = this.uuid
    if( "" !== uuid ) {
      msg.uuid = uuid
    }
    if ( this.remotenode ) this.remotenode.sock.write( message.createmessage( msg ) )
    else this.connection.sock.write( message.createmessage( msg ) )
    msg.timestamp = ( new Date ).getTime()
    this.history.push( msg )

    if( this._sockerr ) {
      nodes.delete( this.remotenode.status.instance )
      try { this.remotenode.sock.close() } catch ( e ) {}
      return false
    }

    return true
  }
}

/* Utils */
function between( min, max ) {
  return Math.floor(
    Math.random() * ( max - min ) + min
  )
}

/* Select a random node based on loadPerCPU
Extend:

We now have nodes which are connected to us and also we have hostname, port pairs which we can connect to.
We need to pick between the 2.

if( nodes.size > 0 ) {
  // do existing stuff
} else
  if( listeningnodes.length ) {
    newnode = pickoneatrandomfromlisteningnodes()
    newnode.connect()
    return newnode
  }
}
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
  async listen() {

    let listenpromiseresolve
    let listenpromise = new Promise( ( r ) => listenpromiseresolve = r )

    this.server = net.createServer( this._onnewconnection.bind( this ) )

    this.server.on( "error", ( e ) => {
      if ( e.code === "EADDRINUSE" ) {
        console.log( "Address in use, retrying..." )
        setTimeout(() => {
          this.server.close()
          this.server.listen( this.port, this.address )
        }, 1000);
      }
    });

    this.server.listen( this.port, this.address )
    this.server.on( "listening", () => listenpromiseresolve( this ) )
    this.server.on( "close", this._oncloseconnection.bind( this ) )

    await listenpromise
  }

  /**
   * Hidden function which is called by our main project RTP when we have servers connected.
   * Private to this module.
   * @private
   */
  openchannel( options, callback ) {
    return new Promise( async ( resolve, reject ) => {
      let request = JSON.parse( JSON.stringify( options ) )
      request.channel = "open"
      let node
      /* Choose our remote node */
      if( options.nodeinstance ) {
        node = nodes.get( options.nodeinstance )
      }

      if( !node ) {
        node = randomenode()
      }

      let r
      if ( nodes.size > 0 ) {
        r = channel._createforlisten( request, node, callback, resolve, reject )
        channels.set( r.id, r )
      } else {
        r = await channel._createforconnect( request, callback, resolve, reject )
      }
    } )
  }

  closenode( node ) {
    var current_node = nodes.get( node )
    if ( !current_node.rejectconnections ) current_node.rejectconnections = true
    if ( current_node.status.channel.current === 0) {
      current_node.sock.destroy()
      nodes.delete( node )
    }
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
        if( this._closingpromiseresolve )
        {
          this._closingpromiseresolve()}
        this._closingpromiseresolve = false
      } )

      if( this._onnewconnectionpromise ) {
        this._onnewconnectionpromise()
        this._onnewconnectionpromise = false
      }

      nodes = new Map()
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

  _onnewconnection( sock ) {

    sock.ournode = {
      "sock": sock,
      "instance": uuidv4(), /* default - which will be overriden */
      "status": {}, /* default - which will be overriden */
      "rejectconnections": false
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
        if( undefined !== receivedmsg.status.instance ) {
          if( !nodes.has( receivedmsg.status.instance ) ) {
            if( nodes.has( sock.ournode.status.instance ) ) {
              nodes.delete( ""+sock.ournode.instance )
            }

            sock.ournode.status.instance = ""+receivedmsg.status.instance 
            nodes.set( sock.ournode.status.instance, sock.ournode )
            if( undefined !== receivedmsg.status ) {
              sock.ournode.status = receivedmsg.status
            }
          }
        }
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

module.exports.listen = async ( port, address, em ) => {

  if( !s ) {
    s = new rtpserver( port, address, em )
    await s.listen()
  }
  return s
}

/**
 * When add node is called, we create a new connection for every (almost) channel we open to a node. This
 * is designed for using within a mesh network (for example docker) where multiple hosts might be behind
 * one listening host.
 * @param { object } node - object contain port and host
 * @param { string } node.host - host name
 * @param { number } node.port - port to connect to
 */
module.exports.addnode = ( node ) => {
  if ( !s ) s = new rtpserver()
  listeningnodes.push( node )
  return s
}
