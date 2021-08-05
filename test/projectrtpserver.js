

const should = require( "chai" ).should()
const expect = require( "chai" ).expect

const projectrtp = require( "../src/build/Release/projectrtp" )


describe( "server", function() {
  describe( "start/stop", function() {
    it( `start and stop the server`, async function() {

      this.timeout( 3000 )

      expect( projectrtp.server.start() ).to.be.an( "object" )

      await new Promise( (resolve) => { setTimeout( resolve, 1500 ) } )
      await projectrtp.server.stop()
    } )
  } )
} )
