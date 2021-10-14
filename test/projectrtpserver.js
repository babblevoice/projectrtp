

const expect = require( "chai" ).expect
const projectrtp = require( "../index.js" ).projectrtp

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
@param {Object} properties
@param {Object} properties.target
@param {number} properties.target.port - the target port - must be an Int and should be even
@param {string} properties.target.address - the target (remote) hostname
@param {number} properties.target.codec - the target codec as a number
@param {Object} [properties.direction] - direction from our perspective
@param {boolean} [properties.direction.send=true]
@param {boolean} [properties.direction.recv=true]
@param {Object} [properties.dtls] - TODO
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

before( () => {
  projectrtp.run()
} )

after( async () => {
  await projectrtp.shutdown()
} )


describe( "server", function() {
  it( `shutdown and run to exist`, async function() {

    expect( projectrtp.shutdown ).to.be.an( "function" )
    expect( projectrtp.run ).to.be.an( "function" )
    expect( projectrtp.openchannel ).to.be.an( "function" )
    expect( projectrtp.stats ).to.be.an( "function" )

  } )

  it( `check stats object`, function( done ) {
    let s = projectrtp.stats()

    /* We are not so much in control of this stat - it needs looking into
    as it is dependant on node releasing the object */
    expect( s.channel.available ).to.be.above( 100 )
    expect( s.channel.current ).to.be.below( 100 )
    done()
  } )
} )
