

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

prtp.projectrtp.run()
testopen()
prtp.projectrtp.shutdown()


