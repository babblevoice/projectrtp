

const net = require( "net" )
const { v4: uuidv4 } = require( "uuid" )

const EventEmitter = require( "events" )

const message = require( "./message" )

/**
 * Node host address and port object
 * @typedef { object } nodehost
 * @property { number } port
 * @property { string } host
 * @property { string } [ instance ]
 */

const nodeconnectiontype = { "listen": 1, "connect": 2 }

/**
 * When we connect to a node track the connection with this object
 * @typedef { object } nodeconnection
 * @property { net.Socket } sock
 * @property { string } instance - the instance id - reported by the node (we provide default until we have the node id)
 * @property { boolean } rejectconnections - no longer accept new channel requests
 * @property { object } status - the most recent status message
 * @property { number } type - nodeconnectiontype
 */


/**
 * @typedef { object } channelbridge
 * @property { channel } left
 * @property { channel } right
 * @property { Set< channel > } channels
 * @ignore
 */
/**
 * @typedef { object } nodebridge
 * @property { Array< channelbridge > } bridges
 * @property { string } main
 * @ignore
 */

/**
 * The remote (listening) nodes
 * @type { Array< nodehost > }
 * @ignore
 */
let listeningnodes = []

/**
 * Our nodes which have connected to our listening server.
 * @type { Map< string, nodeconnection > }
 * @ignore
 */
let nodes = new Map()

/**
 * @type { Map< string, channel > }
 * @ignore
 */
const channels = new Map() /* track where our channels are */

/**
 * TODO
 * Make it pick based more on suitable data such as workercount of node or number of used channels etc.
 * @param { undefined|string } instance - opt for this instance if it exists
 * @returns { nodeconnection }
 * @ignore
 */
function randomenode( instance ) {

  if( instance && nodes.has( instance ) )
    return nodes.get( instance )

  let index = between( 0, nodes.size )
  for( const node of nodes.keys() ) {
    if( 0 === index ) {
      return nodes.get( node )
    }
    index--
  }
}

/**
 * @returns { nodehost }
 * @ignore
 */
function randomconnectnode() {
  return listeningnodes[ between( 0, listeningnodes.length ) ]
}

/**
* @ignore
*/
class channel {

  /** @type { string } */
  id

  /** 
   * @private
   * @type { function } 
   */
  openresolve

  /** 
   * @private
   * @type { function } 
   */
  openreject

  /** 
   * Array of messages sent to and from the remote end.
   * @type { Array< any > } 
   */
  history

  /** 
   * @private
   * @type { object } 
   */
  em

  /**
   * @private
   * @type { boolean }
   */
  _sockerr

  /**
   * @type { nodeconnection }
   */
  connection

  /**
   * Reported from our open channel request and provides port information
   * @type { object }
   */
  channel

  /**
   * Keep track of channels created by one single outbound connection.
   * @type { Array< channel > }
   */
  channels

  /**
   * @private
   * @type { nodebridge }
   */
  _bridge

  /**
   * 
   */
  constructor( id ) {

    if( id ) this.id = id
    else this.id = uuidv4()
    channels.set( this.id, this )

    this.history = []
    /**
     * @member { object }
     * @summary other method for caller to use to receive events from our channel
     */
    this.em = new EventEmitter()

    this._sockerr = false
  }

  /**
 * @typedef { function } channelcallback
 * @param { array } msg - parsed js object
 * @returns { void }
 */

  /**
   * Creates a new channel based on a request to open a channel on a remote
   * node which is already connected to us via the listen mechanism.
   * @param { object } request
   * @param { object } remotenode
   * @param { channelcallback } cb
   * @param { function } openresolve
   * @param { function } openreject
   * @returns { channel }
   * @internal
   */
  static _createforlisten( request, remotenode, cb, openresolve, openreject ) {

    const newchannel = new channel( request.id )

    newchannel.connection = remotenode
    if( cb ) newchannel.em.on( "all", cb )

    newchannel.openresolve = openresolve
    newchannel.openreject = openreject

    newchannel._write( request )

    channels.set( newchannel.id, newchannel )

    return newchannel
  }

  /**
   * Creates a new channel which connects to a remote rtp node.
   * @param { object } options
   * @param { channelcallback } cb
   * @param { function } openresolve
   * @returns { Promise< channel > } - resolves when the connection is connected
   * @internal
   */
  static _createforconnect( options, cb, openresolve /* openreject do something with me */ ) {

    const newchannel = new channel( options.id )
    if( cb ) newchannel.em.on( "all", cb )
    const connecttonode = randomconnectnode()
    return new Promise( connectresolve => {

      newchannel.connection = {
        sock: net.createConnection( connecttonode.port, connecttonode.host ),
        "instance": connecttonode.instance,
        "rejectconnections": false,
        "status": {},
        "type": nodeconnectiontype.connect
      }

      newchannel.connection.sock.setKeepAlive( true )
      newchannel.channels = [ newchannel ]

      newchannel.connection.sock.on( "connect", () => {
        newchannel._write( options )
        if ( connectresolve ) connectresolve( newchannel )
      } )

      const state = message.newstate()
      newchannel.connection.sock.on( "data", ( data ) => {
        message.parsemessage( state, data, ( /** @type { object } */ receivedmsg ) => {

          for ( const chnl of newchannel.channels ) {
            if ( receivedmsg.id === chnl.id ) chnl._on( receivedmsg )
          }

          /* correct the instance id - i.e. in a proxy enviroment (docker swarm) instance may differ */
          if( receivedmsg && receivedmsg.status && receivedmsg.status.instance ) {
            newchannel.connection.instance = receivedmsg.status.instance
          }

          if ( openresolve ) openresolve( newchannel )
        } )
      } )

      newchannel.connection.sock.on( "error", ( e ) => {
        newchannel._sockerr = true
        console.error( e )
      } )

      newchannel.connection.sock.on( "close", () => {
      } )
    } )
  }

  /**
   * This method forces an open channel on the same remote node.
   * @param { object } options
   * @param { channelcallback } cb
   * @returns { Promise< channel > }
   */
  openchannel( options = undefined, cb = undefined ) {
    if( "function" == typeof options ) {
      cb = options
      options = {}
    } else if ( !options ) options = {}

    options.channel = "open"

    const resolvepromise = new Promise( ( resolve, reject ) => {

      if ( 0 < nodes.size ) {
        const node = randomenode( options.nodeinstance )
        const request = JSON.parse( JSON.stringify( options ) )
        request.channel = "open"
        channel._createforlisten( request, node, cb, resolve, reject )
        return
      }

      const newchannel = new channel()
      if( cb ) newchannel.em.on( "all", cb )

      newchannel.connection = this.connection
      newchannel.channels = this.channels
      newchannel.channels.push( newchannel )
      newchannel.openresolve = resolve
      newchannel._write( options )
    } )

    return resolvepromise
  }

  /**
   * @summary Sets or changes the remote of the RTP stream. This can also
   * be passed into the channel.create() object.
   * @param {Object} remote - see channel.create
   * @returns { void }
   */
  remote( remote ) {
    this._write( {
      "channel": "remote",
      "remote": remote
    } )
  }

  /**
   * @summary Close the channel
   */
  close() {

    /* any outstanding to remove where unmix was not called first? */
    //this._removebridge()
    
    /* close us */
    this._write( {
      "channel": "close",
      "uuid": this.uuid
    } )

    if( this.openresolve ) {
      this.openresolve()
      delete this.openresolve
      delete this.openreject
    }
  }

  /**
   * @summary Adds another channel to mix with this one
   * @param {channel} other
   * @returns { Promise< Boolean > }
   */
  async mix( other ) {

    //if( await this._addbridge( other ) ) return true

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
   * Add _bridge object to both channel
   * @param { object } other - other channel
   * @returns { void }
   */
  _createbridges( other ) {

    if( !this._bridge && !other._bridge ) {
      this._bridge = other._bridge = {
        "bridges": [],
        "main": this.connection.instance
      }
    }

    if( !this._bridge && other._bridge ) this._bridge = other._bridge
    if( this._bridge && !other._bridge ) other._bridge = this._bridge

    if( this._bridge !== other._bridge )
      throw new Error( "some complex mixing occured and this function needs finishing" )
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
   * @returns { Promise< boolean > }
   */
  async _addbridge( other ) {

    let leftid = this.connection.instance
    const rightid = other.connection.instance

    this._createbridges( other )

    /* nothing more to do if we are on the same node */
    if( rightid === leftid ) return false

    /* It might have changed now we have linked them up */
    leftid = this._bridge.main

    let found = false
    for( const bridge of this._bridge.bridges ) {
      if( ( bridge.left.connection.instance === leftid || bridge.right.connection.instance === leftid ) &&
          ( bridge.left.connection.instance === rightid || bridge.right.connection.instance === rightid ) ) {
        found = true
        bridge.channels.add( this )
        bridge.channels.add( other )

        this.mix( bridge.left )
        other.mix( bridge.right )
      }
    }

    if( found ) return

    const bridge = {
      /* TODO: this will need reworking with to support reverse connection types */
      "left": await serverinterface.get().openchannel( { "nodeinstance": leftid } ),
      "right": await serverinterface.get().openchannel( { "nodeinstance": rightid } ),
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
   * @returns { boolean }
   */
  _removebridge() {

    /* no remote bridges have been created */
    if( !this._bridge || !this._bridge.main ) return

    const usid = this.connection.instance

    /* The centre of star cannot remove from the network until last man standing */
    if( usid == this._bridge.main ) return

    this._bridge.bridges = this._bridge.bridges.filter( ( bridge ) => {
      bridge.channels.delete( this )

      if( 2 > bridge.channels.size ) {

        bridge.left._write( { "channel": "unmix" } )
        bridge.left._write( { "channel": "unmix" } )

        bridge.left.close()
        bridge.right.close()
        return false /* remove */
      }
      return true /* keep */
    } )
  }

  /**
  @summary Removes us from an existing mix
  @returns { boolean }
  */
  unmix() {
    //this._removebridge()
    this._write( {
      "channel": "unmix",
    } )
    
    return true
  }

  /**
  @summary Send RFC 2833 DTMF digits i.e. channel.dtmf( "#123*" )
  @param { string } digits
  @returns { boolean }
  */
  dtmf( digits ) {
    this._write( {
      "channel": "dtmf",
      "digits": digits
    } )
    return true
  }

  /**
  @summary Echos receved RTP back out when unmixed
  @returns { boolean }
  */
  echo() {
    this._write( {
      "channel": "echo"
    } )
    return true
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
   * @param { soundsoup } soundsoup
   */
  play( soundsoup ){
    this._write( {
      "channel": "play",
      "soup": soundsoup
    } )
    return true
  }

  /**
  @summary Plays audio to the channel when unmixed
  @param { Object } options
  @param { string } options.file - filename of the recording
  @param { number } [ options.startabovepower ] - only start the recording if the average power goes above this value
  @param { number } [ options.finishbelowpower ] - finish the recording if the average power drops below this level
  @param { number } [ options.minduration ] - ensure we have this many mS recording
  @param { number } [ options.maxduration ] - regardless of power options finish when this mS long
  @param { number } [ options.poweraveragepackets ] - number of packets to average the power calcs over
  @param { boolean } [ options.pause ] - pause the recording this function can be called again to pause and resume the recording
  @param { boolean } [ options.finish=false ] - finish the recording
  @returns { boolean }
  */
  record( options ) {
    this._write( {
      "channel": "record",
      "options": options
    } )
    return true
  }

  /**
  @summary Enable/disable the sending and receiving of RTP traffic
  @param { Object } options
  @param { boolean } [ options.send ]
  @param { boolean } [ options.recv ]
  @returns { boolean }
  */
  direction( options ) {
    this._write( {
      "channel": "direction",
      options
    } )
    return true
  }

  /**
   * @private
   * @param { object } msg
   * @returns { boolean } - is further processing required
   */
  _runopen( msg ) {
    if( "open" !== msg.action ) return false

    this.local = msg.local
    this.action = msg.action
    this.uuid = msg.uuid

    if( !this.openresolve ) return true
    this.openresolve( this )
    delete this.openresolve
    delete this.openreject

    return true
  }

  /**
   * 
   * @returns { void }
   * @private
   */
  _runclose() {
    if( undefined !== this.openresolve ) {
      this.openresolve()
      delete this.openresolve
      delete this.openreject
    }

    if( this.channels ) { 
      // adjust with filter
      const index = this.channels.indexOf( this )
      if( -1 < index ) this.channels.splice( index, 1 )
      if( 0 === this.channels.length && this.connection.sock ) this.connection.sock.destroy()
    }


    channels.delete( this.id )
  }

  /**
   * Called by our socket to pass data back up the chain.
   * @ignore
   * @param { object } msg 
   */
  _on( msg ) {
    msg.timestamp = ( new Date ).getTime()
    this.history.push( msg )
    if( this._runopen( msg ) ) return

    this.em.emit( "all", msg )
    this.em.emit( msg.action, msg )

    if( "close" == msg.action ) this._runclose()
  }

  /**
   * Pass a message to the node.
   * @private
   * @param { object } msg 
   * @returns { boolean }
   */
  _write( msg ) {
    msg.id = this.id
    const uuid = this.uuid
    if( "" !== uuid ) {
      msg.uuid = uuid
    }

    this.connection.sock.write( message.createmessage( msg ) )
    
    msg.timestamp = ( new Date ).getTime()
    this.history.push( msg )

    if( this._sockerr && nodeconnectiontype.connect == this.connection.type ) {
      nodes.delete( this.connection.instance )
      try { this.connection.sock.destroy() } catch ( e ) { console.trace( "Unhandled error" )}
      return false
    }

    return true
  }
}

/**
 * @ignore
 */
function between( min, max ) {
  return Math.floor(
    Math.random() * ( max - min ) + min
  )
}

/**
 * @ignore
 */
class rtpserver {

  /**
   * @summary An RTP server. A proxy to remote RTP servers - we can connect (connect) to them
   * or they can connect to us (listen)
   * @hideconstructor
   */

  /**
   * @private
   * @type { function }
   */
  _closingpromiseresolve

  /**
   * @private
   * @type { function }
   */
  _onnewconnectionpromise

  /**
   * 
   * @param { number } port 
   * @param { string } address 
   * @param { object } em 
   */
  constructor( port = -1, address = "", em = undefined ) {
    this.port = port
    this.address = address
    if( em ) this.em = em
  }

  /**
  @summary Listen for new connections.
  @returns { Promise< void > }
  */
  async listen() {

    let listenpromiseresolve
    const listenpromise = new Promise( ( r ) => listenpromiseresolve = r )

    this.server = net.createServer( this._onnewconnection.bind( this ) )

    this.server.on( "error", ( e ) => {
      // It DOES exist
      // @ts-ignore
      if ( "EADDRINUSE" === e.code ) {
        console.log( "Address in use, retrying..." )
        this.server.close()
        setTimeout(() => {
          this.server.listen( this.port, this.address )
        }, 1000 )
      }
    } )

    this.server.listen( this.port, this.address )
    this.server.on( "listening", () => listenpromiseresolve( this ) )
    this.server.on( "close", this._oncloseconnection.bind( this ) )

    await listenpromise
  }

  /**
   * Hidden function which is called by our main project RTP when we have servers connected.
   * @param { object } options
   * @param { channelcallback } callback
   * @returns { Promise< channel > }
   * @internal
   */
  openchannel( options, callback = undefined ) {

    return new Promise( ( resolve, reject ) => {

      if( 0 === nodes.size && 0 === listeningnodes.length ) {
        console.error( "No available RTP nodes" )
        reject( "No available RTP nodes" )
      }

      const request = JSON.parse( JSON.stringify( options ) )
      request.channel = "open"
  
      const node = randomenode( options.nodeinstance )
  
      if ( 0 < nodes.size ) channel._createforlisten( request, node, callback, resolve, reject )
      else channel._createforconnect( request, callback, resolve )

    } )
  }

  /**
   * 
   * @param { string } node 
   */
  closenode( node ) {
    const current_node = nodes.get( node )
    if ( !current_node.rejectconnections ) current_node.rejectconnections = true
    if ( 0 === current_node.status.channel.current) {
      current_node.sock.destroy()
      nodes.delete( node )
    }
  }
  /**
   * @summary Close and clean up
   */
  destroy() {

    serverinterface._clear()

    return new Promise( resolve => {
      this._closingpromiseresolve = resolve
      for ( const [ , node ] of nodes.entries() ) {
        if( node.sock ) node.sock.destroy()
      }

      this.server.close( () => {
        this.server.unref()
        if( this._closingpromiseresolve ) this._closingpromiseresolve()
        delete this._closingpromiseresolve
      } )

      if( this._onnewconnectionpromise ) {
        this._onnewconnectionpromise()
        delete this._onnewconnectionpromise
      }

      nodes = new Map()
    } )
  }

  /**
   * @summary Return a promise which reslolves on the next new connection.
   * Used for testing purposes only - not expected to be used in production.
   * @returns { Promise< void > }
   */
  waitfornewconnection() {
    return new Promise( resolve => {
      if( 0 < nodes.size ) {
        resolve()
        return
      }
      this._onnewconnectionpromise = resolve
    } )
  }

  /**
   * Clean up after we stop lisening
   * @returns { void }
   */
  _oncloseconnection() {
    if( this._closingpromiseresolve ) this._closingpromiseresolve()
    delete this._closingpromiseresolve
  }

  /**
   * Called when a node connects to us.
   * @private
   * @param { net.Socket } sock 
   */
  _onnewconnection( sock ) {

    /** @type { nodeconnection } */
    const ournode = {
      instance: uuidv4(), /* default - which will be overriden */
      sock,
      "rejectconnections": false,
      "status": {},
      "type": nodeconnectiontype.listen
    }

    sock.setKeepAlive( true )
    nodes.set( ournode.instance, ournode )

    sock.on( "close", () => {
      nodes.delete( ""+ournode.instance )
    } )

    sock.on( "error", function ( e ) {
      console.error( e )
      nodes.delete( ournode.instance )
    } )

    const state = message.newstate()

    sock.on( "data", ( data ) => {
      message.parsemessage( state, data, ( receivedmsg ) => {
        /* Have we been told our node instance id */

        if( receivedmsg.status.instance ) {
          if( !nodes.has( receivedmsg.status.instance ) ) {
            if( nodes.has( ournode.instance ) )
              nodes.delete( ournode.instance )

            ournode.instance = receivedmsg.status.instance 
            nodes.set( ournode.instance, ournode )

            if( receivedmsg.status )
              ournode.status = receivedmsg.status
          }
        }
        if( this._onnewconnectionpromise ) {
          this._onnewconnectionpromise()
          delete this._onnewconnectionpromise
        }

        if( undefined !== receivedmsg.id ) {
          if( channels.has( receivedmsg.id ) ) {
            const chan = channels.get( receivedmsg.id )
            chan._on( receivedmsg )
          }
        }
      } )
    } )
  }
}

/**
 * Server Interface. We are the central management server who needs to use remote RTP nodes as our work force. Use this to configure if we are listening for remote ndes to connect to us or we connect to remote nodes.
 * @alias server.interface
 */
class serverinterface {

  /**
   * @private
   * @type { rtpserver }
   */
  static _s

  /**
   * @hideconstructor
   */
  constructor() {
  }

  /**
   * @ignore
   * @returns { serverinterface }
   */
  static create() {
    return new serverinterface()
  }

  /**
   * @summary Get statistics
   * @returns { Object } stats
  */
  stats () {
    return {
      "nodecount": nodes.size
    }
  }

  /**
   * Returns a known list of nodes
   * @returns array< object >
   */
  nodes() {
    const nodesdesc = []

    for ( const [ , node ] of nodes.entries() ) {
      nodesdesc.push( {
        "instance": node.instance,
        "status": node.status
      } )
    }
  
    return nodesdesc
  }

  /**
   * Return underlying rtpserver object - mainly for testing
   * @returns { rtpserver }
   * @ignore
   */
  static get() {
    return serverinterface._s
  }

  /**
   * Listen for a remote node who wishes to connect to us.
   * @param { number } port 
   * @param { string } address 
   * @param { object } em 
   * @returns Promise< rtpserver >
   */
  async listen( port, address, em ) {

    if( serverinterface._s ) return serverinterface._s

    serverinterface._s = new rtpserver( port, address, em )
    await serverinterface._s.listen()

    return serverinterface._s
  }

  /**
   * When add node is called, we create a new connection for every (almost) channel we open to a node. This
   * is designed for using within a mesh network (for example docker) where multiple hosts might be behind
   * one listening host. When configured in this style, we connect to a node when openchannel is called.
   * @param { nodehost } node - object contain port and host
   * @returns { rtpserver }
   */
  addnode( node ) {
    if ( !serverinterface._s ) serverinterface._s = new rtpserver()

    node.instance = uuidv4()
    listeningnodes.push( node )
    return serverinterface._s
  }

  /**
   * Clean up after using remote nodes for us to connect to, switch back to local mode.
   * @returns { void }
   */
  clearnodes = () => {
    delete serverinterface._s
    listeningnodes = []
  }

  /**
   * Remove our reference to server interface
   * @ignore
   */
  static _clear() {
    delete serverinterface._s
  }

  /**
   * Closes any listening ports and cleans up. Called after listen is called 
   * and you need to switch back to local.
   * @returns { Promise< void > }
   */
  static destroy() {
    return serverinterface._s.destroy()
  }
}

module.exports.rtpserver = rtpserver
module.exports.interface = serverinterface
