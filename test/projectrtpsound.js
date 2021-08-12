

const should = require( "chai" ).should()
const expect = require( "chai" ).expect

const projectrtp = require( "../src/build/Release/projectrtp" )


describe( "rtp sound", function() {
  describe( "push/pop", function() {
    it( `push data and check pops and peek happen at the right time`, async function() {


    } )
  } )

  before( () => {
    projectrtp.run()
  } )

  after( async () => {
    await projectrtp.shutdown()
  } )

} )
