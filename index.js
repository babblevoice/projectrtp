
const { v4: uuidv4 } = require( "uuid" )
const server = require( "./lib/server.js" )
const node = require( "./lib/node.js" )

const fs = require( "fs" )
const { spawnSync } = require( "child_process" )

let localaddress = "127.0.0.1"
const bin = "./src/build/Release/projectrtp"

/*
We are using our test files to doc the interface as well as test it as
I can't find any decent toolset to extract this information from c++ comments.
*/

/**
@description
Addon module for an RTP server for audio mixing/recording and playback etc.
*/

/**
@function run
@summary Starts our RTP server
*/

/**
@function shutdown
@summary Shuts down the server, returning a promise which resolves once all tasks are complete.
@returns {Promise}
*/

/**
@function stats
@summary Return an object with the current stats of our server
@returns {stats}
*/

/**
@member soundfile
@type {soundfile}
*/

/**
@member codecx
@type {codecx}
*/

/**
@member tone
@type {tone}
*/

/**
@member wavinfo
@type {wavinfo}
*/

/**
@member proxy
@type {proxy}
*/

/* Generate a self signed if none present */
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
    fs.writeFileSync( combined, serverkeydata + servercertdata )
    fs.unlinkSync( serverkey )
    fs.unlinkSync( servercsr )
    fs.unlinkSync( servercert )
    /* we will be left with combined */
  }
}

/**
@summary Proxy for other RTP nodes
@memberof projectrtp
@hideconstructor
*/
class proxy {
  /**
  @summary Listen for connections from RTP nodes which can offer their services
  to us. When we listen for other nodes, we can configure them so that it is invisible
  to the main node as to where the channel is being handled.
  @param {Object} remote - see channel.create
  @return {rtpserver}
  */
  listen( port = 9002, address = "127.0.0.1" ) {
    return server.listen( port, address )
  }

  /**
  @summary Listen for connections from RTP nodes which can offer their services
  to us. When we listen for other nodes, we can configure them so that it is invisible
  to the main node as to where the channel is being handled.
  @param {Object} remote - see channel.create
  @return {rtpserver}
  */
  stats() {
    return {
      "server": server.stats(),
      "node": {}
    }
  }

  /**
  @summary Returns details of all of the nodes connected to us.
  @return { Object }
  */
  nodes() {
    return server.nodes()
  }

  /**
  @summary Listen for connections from RTP nodes which can offer their services
  to us. When we listen for other nodes, we can configure them so that it is invisible
  to the main node as to where the channel is being handled.
  @param {number} port
  @param {string} host
  @return {rtpnode}
  */
  connect( port = 9002, host = "127.0.0.1" ) {
    return node.connect( module.exports.projectrtp, port, host )
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
@param {Array<string>} [properties.related] - an array of related channel UUID to help in the decision of which node to create the channel on
@param {channelcallback} [callback] - events are passed back to the caller via this callback
@returns {Promise<channel>} - the newly created channel
*/

let actualprojectrtp = false
/**
 * Mimick the underlying napi interface and decide if we need to load the 
 * underlying napi code.
 */
 class projectrtp {

  constructor() {
    this.proxy = new proxy()
  }

  run() {

    if( process.platform == "win32" && process.arch == "x64" ) {
      throw "Platform not currently supported"
    } else if( process.platform == "win32" && process.arch == "ia32" ) {
      throw "Platform not currently supported"
    }

    if( actualprojectrtp ) return

    gencerts()
    actualprojectrtp = require( bin )
    actualprojectrtp.run()
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
  async openchannel( params, cb ) {
    if( "function" == typeof params ) {
      cb = params
      params = {}
    }

    if( "undefined" == typeof params ) params = {}
    if( "undefined" == typeof cb ) cb = ()=>{}

    if( undefined === params.forcelocal &&
        server.get() ) {
      return server.get().openchannel( params, cb )
    } else {
      /* use local */
      let chan = actualprojectrtp.openchannel( params, cb )
      /* I can't find a way of defining a getter in napi - so here we override */
      /* TODO finish address */
      chan.local.address = localaddress

      if( undefined === params.id ) {
        chan.id = uuidv4()
      } else {
        chan.id = params.id
      }

      chan.uuid = uuidv4()
      return chan
    }
  }

  setaddress( address ) {
    localaddress = address
  }
}

module.exports.projectrtp = new projectrtp()
