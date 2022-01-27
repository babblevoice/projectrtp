

const net = require( "net" )
const { v4: uuidv4 } = require( "uuid" )

const message = require( "./message.js" )

let server
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
    this.cb = cb

    this.openresolve = openresolve
    this.openreject = openreject

    this._write( request )
  }

  /**
  @summary Sets or changes the target of the RTP stream. This can also
  be passed into the channel.create() object.
  @param {Object} target - see channel.create
  @return bool
  */
  target( target ){
    this._write( {
      "channel": "target",
      "target": target
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

    channels.delete( this.id )

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
      "other": other
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
  @summary Plays audio to the channel when unmixed
  @param {Object} soundsoup
  @param {Object} soundsoup.files
  @returns {boolean}
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
        @member {Object.target}
        @summary Stores the target information - port and address -
        i.e. send this to the peer so they know where to send RTP to..
        @property {object} target
        @property {string} target.address - the host address
        @property {number} target.port
        @property {object} target.dtls
        @property {string} target.dtls.fingerprint
        @property {boolean} target.dtls.enabled
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
    }
  }

  _write( msg ) {
    msg.id = this.id
    let uuid = this.uuid
    if( "" !== uuid ) {
      msg.uuid = uuid
    }
    this.remotenode.sock.write( message.createmessage( msg ) )
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
  constructor( port, address ) {
    this.port = port
    this.address = address
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
        reject( "No available nodes" )
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

      return
    } )
  }

  /**
  @summary Close and clean up
  */
  destroy() {
    return new Promise( resolve => {
      this.closingpromiseresolve = resolve
      for( let node in nodes ) {
        node.sock.close()
      }

      this.server.close()

      if( undefined !== this._onnewconnectionpromise ) {
        this._onnewconnectionpromise()
        delete this._onnewconnectionpromise
      }

      s = false
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
    this.server.unref()
    this.closingpromiseresolve()
  }
  _onsocketlistening() {
  }

  _onnewconnection( sock ) {

    let newnode = {
      "sock": sock,
      "instance": uuidv4(), /* default - which will be overriden */
      "status": {} /* default - which will be overriden */
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
        if( undefined !== receivedmsg.instance ) {
          if( !nodes.has( receivedmsg.instance ) ) {
            if( nodes.has( newnode.instance ) ) {
              nodes.delete( ""+newnode.instance )
            }

            newnode.instance = receivedmsg.instance
            nodes.set( ""+receivedmsg.instance, newnode )
            if( undefined !== data.status ) {
              newnode.status = data.status
            }

            if( undefined !== this._onnewconnectionpromise ) {
              this._onnewconnectionpromise()
              delete this._onnewconnectionpromise
            }
          }
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

/**
@summary Get statistics
@return {Object} stats
*/
module.exports.stats = () => {
    return {
      "nodecount": nodes.size
    }
}

let s = false
module.exports.get = () => {
  return s
}

module.exports.listen = ( port, address ) => {
  s = new rtpserver( port, address )
  s.listen()
  return s
}
