
const { v4: uuidv4 } = require( "uuid" )
const EventEmitter = require( "events" )

const server = require( "./lib/server.js" )
const node = require( "./lib/node.js" )

const fs = require( "fs" )
const { spawnSync } = require( "child_process" )

let localaddress = "127.0.0.1"
let privateaddress = "127.0.0.1"
const bin = "./src/build/Release/projectrtp"


/**
 * Generate a self signed if none present
 * @return { void }
 */
function gencerts() {

  const keypath = require( "os" ).homedir() + "/.projectrtp/certs/"
  if( !fs.existsSync( keypath + "dtls-srtp.pem" ) ) {

    if ( !fs.existsSync( keypath ) ) fs.mkdirSync( keypath, { recursive: true } )
    
    const serverkey = keypath + "server-key.pem"
    const servercsr = keypath + "server-csr.pem"
    const servercert = keypath + "server-cert.pem"
    const combined = keypath + "dtls-srtp.pem"

    const openssl = spawnSync( "openssl", [ "genrsa", "-out", serverkey, "4096" ] )
    if( 0 !== openssl.status ) throw "Failed to genrsa: " + openssl.status

    const request = spawnSync( "openssl", [ "req", "-new", "-key", serverkey , "-out", servercsr, "-subj", "/C=GB/CN=projectrtp" ] )
    if( 0 !== request.status ) throw "Failed to generate csr: " + request.status

    const sign = spawnSync( "openssl", [ "x509", "-req", "-in", servercsr, "-signkey", serverkey, "-out", servercert ] )
    if( 0 !== sign.status ) throw "Failed to sign key: " + sign.status

    let serverkeydata = fs.readFileSync( serverkey )
    let servercertdata = fs.readFileSync( servercert )
    fs.writeFileSync( combined, Buffer.concat( [ serverkeydata, servercertdata ] ) )
    fs.unlinkSync( serverkey )
    fs.unlinkSync( servercsr )
    fs.unlinkSync( servercert )
    /* we will be left with combined */
  }
}

/**
@summary Proxy for other RTP nodes - to be retired as it is ambiguous of direction (i.e. server/node)
*/
class proxy {

  /**
   * 
   * @param { node.interface } ournode 
   * @param { server.interface } ourserver 
   */
  constructor( ournode, ourserver ) {

    /** @private */
    this._node = ournode
    /** @private  */
    this._server = ourserver
  }

  /**
   * @summary Listen for connections from RTP nodes which can offer their services
   * to us. When we listen for other nodes, we can configure them so that it is invisible
   * to the main node as to where the channel is being handled.
   * @param { Object } port - port to listen on
   * @param { string } address - what address to listen to on
   * @param { object } em - event emitter
   * @return { Promise< server.rtpserver > }
   */
  async listen( em, address = "127.0.0.1", port = 9002 ) {
    return await this._server.listen( port, address, em )
  }

  /**
  @summary Listen for connections from RTP nodes which can offer their services
  to us. When we listen for other nodes, we can configure them so that it is invisible
  to the main node as to where the channel is being handled.
  @return { object }
  */
  stats() {
    return {
      "server": this._server.stats(),
      "node": {}
    }
  }

  /**
  @summary Returns details of all of the nodes connected to us.
  @return { Object }
  */
  nodes() {
    return this._server.nodes()
  }

  /**
   * We are a node and get the connection object.
   * @returns { node.rtpnode }
   */
  node() {
    return this._node.get()
  }

  /**
  @summary Connect node to rtp srvers listening.
  @param {number} port
  @param {string} host
  @return { Promise< node.rtpnode > }
  */
  connect( port = 9002, host = "127.0.0.1" ) {
    return this._node.connect( port, host )
  }

  /**
   * @param { object } node - object contain port and host
   * @param { string } node.host - host name
   * @param { number } node.port - port to connect to
   */
  addnode( node ) {
    return this._server.addnode( node )
  }

  /**
   * Clear current list of nodes (nodes configured for listening)
   */
  clearnodes() {
    this._server.clearnodes()
  }

  get () {
    return server.interface.get()
  }
}

/**
Callback for events we pass back to interested parties.
@callback channelcallback
@param {object} event
@listens close
@listens record
@listens play
@listens telephone-event
*/

/**
Channel closed event
@event close
@type {object}
@property {string} action - "close"
@property {string} reason - the reason the channel was closed
@property {object} tick - statistics related to the channels interval timer
@property {object} in - in traffic statistics
@property {object} out - out traffic statistics
*/

/**
Channel recording events
@event record
@type {object}
@property {string} action - "record"
@property {string} file - filename of the recording
@property {string} event - details of what happened
*/

/**
Events related to sound file playback
@event play
@type {object}
@property {string} action - "play"
@property {string} event - what the event was
@property {string} reason - more details regarding the event
*/

/**
RFC 2833 telephone-event
@event telephone-event
@type {object}
@property {string} action - "telephone-event"
@property {string} event - the DTMF character pressed
*/

/**
@function openchannel
@summary Opens a channel and returns a channel object.
@param {Object} [properties]
@param {string} [properties.id] Unique id provided which is simply returned in the channel object
@param {Object} [properties.remote]
@param {number} properties.remote.port - the remote port - must be an Int and should be even
@param {string} properties.remote.address - the remote (remote) host address
@param {number} properties.remote.codec - the remote codec as a number
@param {Object} [properties.remote.dtls]
@param {string} properties.remote.dtls.fingerprint - the fingerprint we verify the remote against
@param {string} properties.remote.dtls.setup - "active" or "passive"
@param {Object} [properties.direction] - direction from our perspective
@param {boolean} [properties.direction.send=true]
@param {boolean} [properties.direction.recv=true]
@param {channelcallback} [callback] - events are passed back to the caller via this callback
@return {Promise<channel>} - the newly created channel
*/

let actualprojectrtp
/**
 * Mimick the underlying napi interface and decide if we need to load the 
 * underlying napi code.
 */
class projectrtp {

  constructor() {
    /*
      Expose our node and server interface. node is the rtp node, server is the control server (i.e. sip)
     */
    this.node = node.interface.create( this )
    this.server = server.interface.create()

    /* to be retired - it is now confusing on direction to maintain in one interface */
    this.proxy = new proxy( this.node, this.server )
  }

  /**
   * 
   * @param { object|undefined } params 
   * @returns { void }
   */
  run( params ) {

    if( process.platform == "win32" && process.arch == "x64" ) {
      throw "Platform not currently supported"
    } else if( process.platform == "win32" && process.arch == "ia32" ) {
      throw "Platform not currently supported"
    }

    if( actualprojectrtp ) return

    gencerts()
    if ( !params )
    {
      params = {}
    }
    actualprojectrtp = require( bin )
    actualprojectrtp.run( params )
    this.dtls = actualprojectrtp.dtls
    this.tone = actualprojectrtp.tone
    this.rtpfilter = actualprojectrtp.rtpfilter
    this.codecx = actualprojectrtp.codecx
    this.soundfile = actualprojectrtp.soundfile
    this.rtpbuffer = actualprojectrtp.rtpbuffer
    this.stats = actualprojectrtp.stats
    this.shutdown = actualprojectrtp.shutdown
  }

  /**
   * 
   * @param { object } params 
   * @param { channelcallback } cb 
   */
  async openchannel( params = undefined, cb = undefined ) {
    if( "function" == typeof params ) {
      cb = params
      params = {}
    }

    if( "undefined" == typeof params ) params = {}

    if( undefined === params.forcelocal && server.interface.get() ) {
      return server.interface.get().openchannel( params, cb )
    } else {
      /* use local */
      let chan = actualprojectrtp.openchannel( params, ( d ) => {
        try{
          if( chan.em ) {
            chan.em.emit( "all", d )
            if( d.action ) chan.em.emit( d.action, d )
          }
        } catch ( e ) {
          console.trace( e )
        }
      } )
      /* I can't find a way of defining a getter in napi - so here we override */

      chan.local.address = localaddress
      chan.local.privateaddress = privateaddress

      chan.em = new EventEmitter()
      if( cb ) chan.em.on( "all", cb )

      if( undefined === params.id ) {
        chan.id = uuidv4()
      } else {
        chan.id = params.id
      }

      chan.uuid = uuidv4()
      return chan
    }
  }

  /**
   * 
   * @param { string } address
   * @returns { void } 
   */
  setaddress( address ) {
    localaddress = address
  }

  /**
   * 
   * @param { string } address
   * @returns { void }
   */
  setprivateaddress( address ) {
    privateaddress = address
  }
}

module.exports.projectrtp = new projectrtp()
