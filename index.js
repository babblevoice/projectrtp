
const { v4: uuidv4 } = require( "uuid" )

/* TODO */

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
@function openchannel
@summary Opens a channel and returns a channel object.
@param {Object} [properties]
@param {string} [properties.id] Unique id provided which is simply returned in the channel object
@param {Object} [properties.target]
@param {number} properties.target.port - the target port - must be an Int and should be even
@param {string} properties.target.address - the target (remote) hostname
@param {number} properties.target.codec - the target codec as a number
@param {Object} [properties.direction] - direction from our perspective
@param {boolean} [properties.direction.send=true]
@param {boolean} [properties.direction.recv=true]
@param {Object} [properties.dtls]
@param {string} properties.dtls.fingerprint - the fingerprint we verify the remote against
@param {string} properties.dtls.setup - "act" or "pass"
@param {function} [callback] - events are passed back to the caller via this callback
@returns {channel} - the newly created channel
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


if( process.platform == "win32" && process.arch == "x64" ) {
  throw "Platform not currently supported"
} else if( process.platform == "win32" && process.arch == "ia32" ) {
  throw "Platform not currently supported"
}


module.exports.projectrtp = require( "./src/build/Release/projectrtp" )

/*
Wrap our createchannel function to simplify some of the calling
and make param and cb optional.
This is also where we will hook into to pass requests over to
remote projectrtp nodes.
*/
let oc = module.exports.projectrtp.openchannel
Object.defineProperty( module.exports.projectrtp, "openchannel", {
  value: function( x, y ) {

    let params = x
    let cb = y
    if( "function" == typeof x ) {
      cb = params
      params = {}
    }

    if( "undefined" == typeof params ) params = {}
    if( "undefined" == typeof cb ) cb = ()=>{}

    let chan = oc( params, cb )

    /* I can't find a way of defining a getter in napi - so here we override */
    let port = chan.port
    Object.defineProperty( chan, "port", {
        get: function () { return port }
    } )

    if( undefined !== params.id ) {
      chan.id = params.id
    }

    chan.uuid = uuidv4()

    return chan
  }
} )
