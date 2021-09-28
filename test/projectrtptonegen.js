/*
TODO
Call the function and check we have output files. Further testing.
*/

const expect = require( "chai" ).expect

let projectrtp
if( "debug" === process.env.build ) {
  projectrtp = require( "../src/build/Debug/projectrtp" )
} else {
  projectrtp = require( "../src/build/Release/projectrtp" )
}

describe( "tonegen", function() {
  it( `tone.generate exists`, async function() {
    expect( projectrtp.tone.generate ).to.be.an( "function" )
  } )
} )
