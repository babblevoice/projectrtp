

const expect = require( "chai" ).expect

let projectrtp
if( "debug" === process.env.build ) {
  projectrtp = require( "../src/build/Debug/projectrtp" )
} else {
  projectrtp = require( "../src/build/Release/projectrtp" )
}


describe( "server", function() {
  it( `shutdown and run to exist`, async function() {

    expect( projectrtp.shutdown ).to.be.an( "function" )
    expect( projectrtp.run ).to.be.an( "function" )

  } )
} )
