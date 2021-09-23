
let projectrtp
if( "debug" === process.env.build ) {
  projectrtp = require( "../src/build/Debug/projectrtp" )
} else {
  projectrtp = require( "../src/build/Release/projectrtp" )
}

before( () => {
  projectrtp.run()
} )

after( async () => {
  await projectrtp.shutdown()
} )
