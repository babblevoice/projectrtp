
const should = require( "chai" ).should()
const expect = require( "chai" ).expect

const projectrtp = require( "../src/build/Release/projectrtp" )

describe( "rtpchannel", function() {

  it( `structure of rtpchannel is correct`, async function() {

    expect( projectrtp.rtpchannel ).to.be.an( "object" )
    expect( projectrtp.rtpchannel.create ).to.be.an( "function" )

  } )

  it( `call create channel and check the structure of the returned object`, async function() {

    let channel = projectrtp.rtpchannel.create()
    expect( channel ).to.be.an( "object" )

    expect( channel.close ).to.be.an( "function" )
    expect( channel ).to.have.property( "port" ).that.is.a( "number" )

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 1000 ) } )
    channel.close()
  } )

  it( `call create channel echo`, async function() {
    let channel = projectrtp.rtpchannel.create()
    expect( channel ).to.be.an( "object" )

    expect( channel.close ).to.be.an( "function" )
    expect( channel ).to.have.property( "port" ).that.is.a( "number" )

    let s = channel.echo()
    expect( s ).to.be.an( "boolean" )

    channel.close()
  } )

  before( () => {
    projectrtp.run()
  } )

  after( async () => {
    await projectrtp.shutdown()
  } )
} )
