// @ts-nocheck
// no check as this file is mainly here for documentation - and only a simple test is included.

const expect = require( "chai" ).expect
const projectrtp = require( "../../index.js" ).projectrtp

describe( "tonegen", function() {
  it( "tone.generate exists", async function() {
    expect( projectrtp.tone.generate ).to.be.an( "function" )
  } )
} )

