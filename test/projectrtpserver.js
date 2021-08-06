

const should = require( "chai" ).should()
const expect = require( "chai" ).expect

const projectrtp = require( "../src/build/Release/projectrtp" )


describe( "server", function() {
  describe( "shutdown", function() {
    it( `shutdown to exist`, async function() {

      expect( projectrtp.shutdown ).to.be.an( "function" )

    } )
  } )

  before( () => {
    projectrtp.run()
  } )

  after( async () => {
    await projectrtp.shutdown()
  } )
} )
