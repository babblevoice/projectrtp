

const expect = require( "chai" ).expect
const projectrtp = require( "../../index.js" ).projectrtp

before( () => {
  projectrtp.run()
} )

after( async () => {
  await projectrtp.shutdown()
} )

/**
 * Test we ensure that the server has cleaned up that channel's resources.
 * This hook runs after every test in every file. A test that failed part-way
 * may not have reached its close calls: its channels are closed here instead,
 * because a failing afterEach makes mocha skip every test after it.
 */
afterEach( async function() {
  if( "failed" === this.currentTest.state ) {
    await projectrtp.shutdown()
    return
  }
  const stats = projectrtp.stats()
  expect( stats.channel.current ).to.equal( 0 )
} )


describe( "server", function() {
  it( "shutdown and run to exist", async function() {

    expect( projectrtp.shutdown ).to.be.an( "function" )
    expect( projectrtp.run ).to.be.an( "function" )
    expect( projectrtp.openchannel ).to.be.an( "function" )
    expect( projectrtp.stats ).to.be.an( "function" )

  } )
} )
