

const prtp = require( "../index.js" )

/**
 * Very basic test - to run in release workflow before we install 
 * dev dependancies - i.e. check we can run without dev dependancies.
 */

/**
 * Test open a channel.
 */
async function testopen() {

  
  const chan = await prtp.projectrtp.openchannel()

  try{
    if( !chan ) throw new Error( "Bad channel" )
    if( "number" !== typeof chan.local.port ) throw new Error( "Port doesn't look correct" )
  } finally {
    chan.close()
  }
}

let retval = 0
console.log( "Running project RTP" )
prtp.projectrtp.run()

console.log( "Test open channel" )

try{
  testopen()
} catch( e ) {
  console.log( e )
  retval = 1
}

console.log( "Shutting down projectrtp" )
prtp.projectrtp.shutdown()
console.log( "Shutdown - good to proceed" )

process.exit( retval )


