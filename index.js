
const { v4: uuidv4 } = require( "uuid" )
const server = require( "./lib/server.js" )
const node = require( "./lib/node.js" )


/*
We are using our test files to doc the interface as well as test it as
I can't find any decent toolset to extract this informaton from c++ comments.
*/

/**
@module projectrtp
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


if( process.platform == "win32" && process.arch == "x64" ) {
  throw "Platform not currently supported"
} else if( process.platform == "win32" && process.arch == "ia32" ) {
  throw "Platform not currently supported"
}


module.exports.projectrtp = require( "./src/build/Release/projectrtp" )

/*
Wrap our openchannel function to simplify some of the calling
and make param and cb optional.
This is also where we will hook into to pass requests over to
remote projectrtp nodes if we have remote nodes rather than local addon.
*/
/**
@function openchannel
@summary Opens a channel and returns a channel object.
@param {Object} [properties]
@param {string} [properties.id] Unique id provided which is simply returned in the channel object
@param {Object} [properties.target]
@param {number} properties.target.port - the target port - must be an Int and should be even
@param {string} properties.target.address - the target (remote) hostname
@param {number} properties.target.codec - the target codec as a number
@param {Object} [properties.target.dtls]
@param {string} properties.target.dtls.fingerprint - the fingerprint we verify the remote against
@param {string} properties.target.dtls.setup - "act" or "pass"
@param {Object} [properties.direction] - direction from our perspective
@param {boolean} [properties.direction.send=true]
@param {boolean} [properties.direction.recv=true]
@param {Array<string>} [properties.related] - an array of related channel UUID to help in the decision of which node to create the channel on
@param {function} [callback] - events are passed back to the caller via this callback
@returns {Promise<channel>} - the newly created channel
*/
let oc = module.exports.projectrtp.openchannel
Object.defineProperty( module.exports.projectrtp, "openchannel", {
  value: async function( x, y ) {

    let params = x
    let cb = y
    if( "function" == typeof x ) {
      cb = params
      params = {}
    }

    if( "undefined" == typeof params ) params = {}
    if( "undefined" == typeof cb ) cb = ()=>{}

    if( server.stats().nodecount > 0 && server.get() ) {
      return server.get().openchannel( params, cb )
    } else {
      /* use local */
      let chan = oc( params, cb )
      /* I can't find a way of defining a getter in napi - so here we override */
      let port = chan.local.port
      /* TODO finish IP */
      Object.defineProperty( chan, "local", {
          get: function () {
            return {
              "port": port,
              "address": ""
            }
          }
      } )

      if( undefined === params.id ) {
        chan.id = uuidv4()
      } else {
        chan.id = params.id
      }

      chan.uuid = uuidv4()
      return chan
    }
  }
} )

let ss = module.exports.projectrtp.stats
Object.defineProperty( module.exports.projectrtp, "stats", {
  value: function() {
    return ss()
  }
} )

/**
@summary Proxy for other RTP nodes
@memberof projectrtp
@hideconstructor
*/
class proxy {
  /**
  @summary Listen for connections from RTP nodes which can offer their services
  to us. When we listen for other nodes, we can configure them so that it is invisable
  to the main node as to where the channel is being handled.
  @param {Object} target - see channel.create
  @return {rtpserver}
  */
  listen( port = 9002, address = "127.0.0.1" ) {
    return server.listen( module.exports.projectrtp, port, address )
  }

  /**
  @summary Listen for connections from RTP nodes which can offer their services
  to us. When we listen for other nodes, we can configure them so that it is invisable
  to the main node as to where the channel is being handled.
  @param {Object} target - see channel.create
  @return {rtpserver}
  */
  stats() {
    return {
      "server": server.stats(),
      "node": {}
    }
  }

  /**
  @summary Listen for connections from RTP nodes which can offer their services
  to us. When we listen for other nodes, we can configure them so that it is invisable
  to the main node as to where the channel is being handled.
  @param {number} port
  @param {string} host
  @return {rtpnode}
  */
  connect( port=9002, host="127.0.0.1" ) {
    return node.connect( module.exports.projectrtp, port, host )
  }
}

module.exports.projectrtp.proxy = new proxy()
