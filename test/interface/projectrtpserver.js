

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
} )
