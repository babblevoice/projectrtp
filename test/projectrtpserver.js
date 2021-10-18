

const expect = require( "chai" ).expect
const projectrtp = require( "../index.js" ).projectrtp

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
