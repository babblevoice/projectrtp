

const net = require( "net" )
const { v4: uuidv4 } = require( "uuid" )

const EventEmitter = require( "events" )

const message = require( "./message.js" )

/**
 * Node host address and port object
 * @typedef { object } nodehost
 * @property { number } port
 * @property { string } host
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
 */
/**
 * @typedef { object } nodebridge
 * @property { Array< channelbridge > } bridges
 * @property { string } main
 */

/**
 * The remote (listening) nodes
 * @type { Array< nodehost > }
 */
let listeningnodes = []

/**
 * Our nodes which have connected to our listening server.
 * @type { Map< string, nodeconnection > }
 */
let nodes = new Map()

/**
 * @type { Map< string, channel > } 
 */
let channels = new Map() /* track where our channels are */

/**
 * TODO
 * Make it pick based more on suitable data such as workercount of node or number of used channels etc.
 * @param { undefined|string } instance - opt for this instance if it exists
 * @returns { nodeconnection } 
 */
function randomenode( instance ) {

  if( instance && nodes.has( instance ) )
    return nodes.get( instance )

  let index = between( 0, nodes.size )
  for( let node of nodes.keys() ) {
    if( 0 === index ) {
      return nodes.get( node )
    }
    index--
  }
}

/**
 * @returns { nodehost }
 */
function randomconnectnode() {
  return listeningnodes[ between( 0, listeningnodes.length ) ]
}

/*
This class definition serves both to document the channel
definition in our addon and also to to reflect te remote
channel in a remote node.
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
  constructor() {

    this.id = uuidv4()
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
   * Creates a new channel based on a request to open a channel on a remote
   * node which is already connected to us via the listen mechanism.
   * @param { object } request
   * @param { object } remotenode
   * @param { ( ...args: any[]) => void } cb
   * @param { function } openresolve
   * @param { function } openreject
   * @returns { channel }
   * @internal
   */
  static _createforlisten( request, remotenode, cb, openresolve, openreject ) {

    let newchannel = new channel()
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
   * @param { ( ...args: any[]) => void } cb
   * @param { function } openresolve
   * @returns { Promise< channel > } - resolves when the connection is connected
   * @internal
   */
  static _createforconnect( options, cb, openresolve /* openreject do something with me */ ) {

    let newchannel = new channel()
    if( cb ) newchannel.em.on( "all", cb )
    let connecttonode = randomconnectnode()
    return new Promise( connectresolve => {

      newchannel.connection = {
        sock: net.createConnection( connecttonode.port, connecttonode.host ),
        "instance": uuidv4(),
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

      let state = message.newstate()
      newchannel.connection.sock.on( "data", ( data ) => {
        message.parsemessage( state, data, ( receivedmsg ) => {
          for ( let chnl of newchannel.channels ) {
            if ( receivedmsg.id === chnl.id ) chnl._on( receivedmsg )
          }

          /* find channel */
          if( openresolve && receivedmsg.action == "open" ) {
            Object.assign( newchannel, receivedmsg )
            openresolve( newchannel )
          }
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
   * @returns { Promise< channel > }
   */
  openchannel( options = {} ) {

    options.channel = "open"

    const resolvepromise = new Promise( ( res ) => {

      let newchannel = new channel()
      newchannel.connection = this.connection
      newchannel.channels = this.channels
      newchannel.channels.push( newchannel )
      newchannel._write( options )
      res( newchannel )
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
    this._removebridge()

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

    let leftid = this.connection.instance
    const rightid = other.connection.instance

    if( !this._bridge && !other._bridge ) {
      this._bridge = other._bridge = {
        "bridges": [],
        "main": leftid
      }
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

    let bridge = {
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
   * @returns 
   */
  _removebridge() {

    /* no remote bridges have been created */
    if( !this._bridge || !this._bridge.main ) return

    const usid = this.connection.instance

    /* The centre of star cannot remove from the network until last man standing */
    if( usid == this._bridge.main ) return

    this._bridge.bridges = this._bridge.bridges.filter( ( bridge ) => {
      bridge.channels.delete( this )

      if( bridge.channels.size < 2 ) {

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
   * Called by our socket to pass data back up the chain.
   * @internal
   * @param { object } msg 
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

      if( this.channels ) { 
        // adjust with filter
        const index = this.channels.indexOf( this )
        if( index > -1) this.channels.splice(index, 1)
        if( this.channels.length === 0 && this.connection.sock) this.connection.sock.destroy()
      }
  
      if( undefined !== this.openresolve ) {
        this.openresolve()
        delete this.openresolve
        delete this.openreject
      }
      if( this.connection && this.connection.sock && !this.channels ) {
        this.connection.sock.destroy()
        delete this.connection.sock
      }

      channels.delete( this.id )
    } 
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

/* Utils */
function between( min, max ) {
  return Math.floor(
    Math.random() * ( max - min ) + min
  )
}

/**
@summary An RTP server. A proxy to remote RTP servers - we can connect (connect) to them
or they can connect to us (listen)
@hideconstructor
*/
class rtpserver {

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
    this.em = em
  }

  /**
  @summary Listen for new connections.
  @returns { Promise< void > }
  */
  async listen() {

    let listenpromiseresolve
    let listenpromise = new Promise( ( r ) => listenpromiseresolve = r )

    this.server = net.createServer( this._onnewconnection.bind( this ) )

    this.server.on( "error", ( e ) => {
      // It DOES exist
      // @ts-ignore
      if ( e.code === "EADDRINUSE" ) {
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
   * @param { ( ...args: any[]) => void } callback
   * @returns { Promise< channel > }
   * @internal
   */
  openchannel( options, callback = undefined ) {

    return new Promise( ( resolve, reject ) => {

      if( 0 === nodes.size && 0 === listeningnodes.length ) {
        console.error( "No available RTP nodes" )
        reject( "No available RTP nodes" )
      }

      let request = JSON.parse( JSON.stringify( options ) )
      request.channel = "open"
  
      const node = randomenode( options.nodeinstance )
  
      if ( nodes.size > 0 ) channel._createforlisten( request, node, callback, resolve, reject )
      else channel._createforconnect( request, callback, resolve )

    } )
  }

  /**
   * 
   * @param { string } node 
   */
  closenode( node ) {
    var current_node = nodes.get( node )
    if ( !current_node.rejectconnections ) current_node.rejectconnections = true
    if ( current_node.status.channel.current === 0) {
      current_node.sock.destroy()
      nodes.delete( node )
    }
  }
  /**
   * @summary Close and clean up
   */
  destroy() {

    delete serverinterface._s

    return new Promise( resolve => {
      this._closingpromiseresolve = resolve
      for ( const [ , node ] of nodes.entries() ) {
        node.sock.destroy()
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
      if( nodes.size > 0 ) {
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
    let ournode = {
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

    let state = message.newstate()

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
            let chan = channels.get( receivedmsg.id )
            chan._on( receivedmsg )
          }
        }
      } )
    } )
  }
}

class serverinterface {

  /**
   * @internal
   * @type { rtpserver }
   */
  static _s

  constructor() {
  }

  /**
   * 
   * @returns { serverinterface }
   */
  static create() {
    return new serverinterface()
  }

  /**
   * @summary Get statistics
   * @returns {Object} stats
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
    let nodesdesc = []

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
   */
  static get() {
    return serverinterface._s
  }

  /**
   * 
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
   * one listening host.
   * @param { nodehost } node - object contain port and host
   * @returns { rtpserver }
   */
  addnode( node ) {
    if ( !serverinterface._s ) serverinterface._s = new rtpserver()
    listeningnodes.push( node )
    return serverinterface._s
  }

  /**
   * @returns { void }
   */
  clearnodes = () => {
    listeningnodes = []
  }
}

module.exports.rtpserver = rtpserver
module.exports.interface = serverinterface
