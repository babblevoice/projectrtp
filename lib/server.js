

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
@param {string} local.address - Local adress of the channel
@param {number} local.port - Local port of the channel
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
    this.cb = cb
    this.nodebridges = []
    this.mixes = [ this ]
    /**
     * @member { object }
     * @summary other method for caller to use to receive events from our channel
     */
    this.em = new EventEmitter()
    if( cb ) this.em.on( "all", cb )

    this.openresolve = openresolve
    this.openreject = openreject
    this._sockerr = false

    this.remotenode.sock.on( "error", ( e ) => {
      this._sockerr = true 
      console.error( e )
    } )

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
  async mix( other ) {
    if( other.remotenode.status.instance === this.remotenode.status.instance )
    {
      // Channels are on the same node
      this._write( {
        "channel": "mix",
        "other": {
          "id": other.id,
          "uuid": other.uuid
        }
      } )
      combinemixes( this.mixes, other.mixes )
      // It is a local mix ( same node ), but we still need to update nodebridges reference
      this._addnodebridge( other, false )
    }
    else
    {
      // Channels are on different nodes
      for ( let nodebridge in other.nodebridges )
      {
        // Look for an existing connection and mix with the local channel
        if ( nodebridge.other.node === this.remotenode.status.instance )
        {
          this._write( {
            "channel": "mix",
            "other": {
              "id": nodebridge.other.channels[0].id,
              "uuid": nodebridge.other.channels[0].uuid
            }
          } )
          combinemixes( this.mixes, nodebridge.other.channels )
          this._addnodebridge( other, false )
          return
        }
      }
      // Create a bridge between 2 nodes and mix
      var bridge1 = await s.openchannel( { "nodeinstance": this.remotenode.status.instance }, this.cb )
      var bridge2 = await s.openchannel( { "nodeinstance": other.remotenode.status.instance }, other.cb )
      bridge1.remote({ "address": bridge2.local.address, "port": bridge2.local.port, "codec": this.codec })
      bridge2.remote({ "address": bridge1.local.address, "port": bridge1.local.port, "codec": other.codec })

      bridge1._write( {
        "channel": "mix",
        "other": {
          "id": this.id,
          "uuid": this.uuid
        }
      } )
      bridge2._write( {
        "channel": "mix",
        "other": {
          "id": other.id,
          "uuid": other.uuid
        }
      } )

      this._addnodebridge( other, { "bridge1": bridge1, "bridge2": bridge2 } )
    }
  }

// Private function for dealing with nodebridges
_addnodebridge( other, remote )
{
  // remote = false ( if mixing locally ), remote = { "bridge1": bridge1, "bridge2": bridge2 } ( if mixing remotely )
  // Both channels are mains, so transfer main to *this*
  if ( this.nodebridges.length !=0 && other.nodebridges.length != 0 )
  {
    if ( remote )
    {
      this.nodebridges.push({
        "main": {
          "node": this.remotenode.status.instance,
          "channels": this.mixes,
          "bridge": remote.bridge1
        },
        "other": {
          "node": other.remotenode.status.instance,
          "channels": other.mixes,
          "bridges": remote.bridge2
        }
      })
    }
    for ( let nodebridge of other.nodebridges )
    {
      nodebridge.main.node = this.remotenode.status.instance
      nodebridge.main.mixes = this.mixes
      this.nodebridges.push( nodebridge )
    }
    other.nodebridges = this.nodebridges
    for ( let chnl of other.mixes )
    {
      chnl.nodebridges = this.nodebridges
    }
  }
  // Other is main
  else if ( other.nodebridges.length != 0 )
  {
    if ( remote )
    {
      other.nodebridges.push({
        "main": {
          "node": other.remotenode.status.instance,
          "channels": this.mixes,
          "bridge": remote.bridge2
        },
        "other": {
          "node": this.remotenode.status.instance,
          "channels": other.mixes,
          "bridge": remote.bridge1
        }
      })
    }
    this.nodebridges = other.nodebridges
    for ( let chnl of this.mixes )
    {
      chnl.nodebridges = other.nodebridges
    }
  }
  // *this* is main
  else if ( this.nodebridges.length != 0 )
  {
    if ( remote )
    {
      this.nodebridges.push({
        "main": {
          "node": this.remotenode.status.instance,
          "channels": this.mixes,
          "bridge": remote.bridge1
        },
        "other": {
          "node": other.remotenode.status.instance,
          "channels": other.mixes,
          "bridge": remote.bridge2
        }
      })
    }
    other.nodebridges = this.nodebridges
    for ( let chnl of other.mixes )
    {
      chnl.nodebridges = this.nodebridges
    }
  }
  // None of them is main, so select *this* as main
  else if ( remote )
  {
    this.nodebridges.push({
      "main": {
        "node": this.remotenode.status.instance,
        "channels": this.mixes,
        "bridge": remote.bridge1
      },
      "other": {
        "node": other.remotenode.status.instance,
        "channels": other.mixes,
        "bridge": remote.bridge2
      }
    })
    other.nodebridges = this.nodebridges
    for ( let chnl of this.mixes )
    {
      chnl.nodebridges = this.nodebridges
    }
    for ( let chnl of other.mixes )
    {
      chnl.nodebridges = other.nodebridges
    }
  }
  // Make sure to update nodebridges references for locally mixed channels
  else
  {
    for ( let chnl of this.mixes )
    {
      chnl.nodebridges = this.nodebridges
    }
    for ( let chnl of other.mixes )
    {
      chnl.nodebridges = other.nodebridges
    }
  }
}

  /**
  @summary Removes us from an existing mix
  @param {channel} other
  @returns {boolean}
  */
  unmix(){
    for ( let nodebridge of this.nodebridges )
    {
      if ( nodebridge.other.node === this.remotenode.status.instance && nodebridge.main.node != this.remotenode.status.instance )
      {
        const index = nodebridge.other.channels.indexOf( this );
        if ( index > -1 )
        {
          nodebridge.other.channels.splice(index, 1);
        }
        if ( nodebridge.other.channels.length == 0 )
        {
          nodebridge.other.bridge.unmix()
          nodebridge.other.bridge.close()
          nodebridge.main.bridge.unmix()
          nodebridge.main.bridge.close()
        }
        break
      }
    }
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
  direction( options ){}

  _on( msg ) {
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
    if( this._sockerr ) {
      nodes.delete( this.remotenode.instance )
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
function combinemixes( mix1, mix2 ) {
  for( let chnl of mix2)
  {
    mix1.push( chnl )
    chnl.mixes = mix1
  }
  for ( let chnl of mix1 )
  {
    chnl.mixes = mix1
  }
}

/* Select a random node based on loadPerCPU */
function randomenode() {
  var min = 100000
  var min_node
  for( let node of nodes.keys() ) 
  {
    var current_node = nodes.get( node )
    if ( Object.keys( current_node.status ).length !== 0)
    {
      if ( !current_node.rejectconnections )
      {
        let loadPerCPU = current_node.status.channel.current /  current_node.status.workercount
        if ( loadPerCPU < min )
        {
          min = loadPerCPU
          min_node = current_node
        }
      }
    }
  }
  if (min_node === undefined)
  {
    let index = between( 0, nodes.size )
    for( let node of nodes.keys() ) {
      if( 0 === index ) {
        return nodes.get( node )
      }
      index--
  }
  }
  return min_node
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
      if (options.nodeinstance === undefined)
      {
        if ( undefined === related) {
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
      }
      else
      {
        node = nodes.get( options.nodeinstance )
      }

      let r = new channel( request, node, callback, resolve, reject )
      channels.set( r.id, r )
      return
    } )
  }

  closenode( node ) {
    var current_node = nodes.get( node )
    if ( !current_node.rejectconnections ) current_node.rejectconnections = true
    if ( current_node.status.channel.current === 0)
    {
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

    let newnode = {
      "sock": sock,
      "instance": uuidv4(), /* default - which will be overriden */
      "status": {}, /* default - which will be overriden */
      "rejectconnections": false
    }
    sock.setKeepAlive( true )
    nodes.set( newnode.instance, newnode )

    sock.on( "close", () => {
      nodes.delete( ""+newnode.instance )
    } )

    let state = message.newstate()

    sock.on( "data", ( data ) => {
      message.parsemessage( state, data, ( receivedmsg ) => {
        /* Have we been told our node instance id */
        if( undefined !== receivedmsg.status.instance ) {
          if( !nodes.has( receivedmsg.status.instance ) ) {
            if( nodes.has( newnode.instance ) ) {
              nodes.delete( ""+newnode.instance )
            }
            newnode.instance = ""+receivedmsg.status.instance
            nodes.set( newnode.instance, newnode )
          }
          
          if( undefined !== receivedmsg.status ) {
            newnode.status = receivedmsg.status
          }
          if( this._onnewconnectionpromise ) {
            this._onnewconnectionpromise()
            this._onnewconnectionpromise = false
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
