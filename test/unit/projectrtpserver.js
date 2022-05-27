

const expect = require( "chai" ).expect
const projectrtp = require( "../../index.js" ).projectrtp

describe( "server", function() {

  it( `check stats object`, function( done ) {
    let s = projectrtp.stats()

    /* We are not so much in control of this stat - it needs looking into
    as it is dependant on node releasing the object */
    expect( s.channel.available ).to.be.above( 100 )
    expect( s.channel.current ).to.be.below( 100 )
    done()
  } )
} )
