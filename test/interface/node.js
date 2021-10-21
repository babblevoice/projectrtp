const expect = require( "chai" ).expect

const prtp = require( "../../index.js" ).projectrtp

describe( "node", function() {

  it( `connect to server`, async function() {

    let proxy = prtp.proxy.listen()
    let n = await prtp.proxy.connect()

    await proxy.waitfornewconnection()
    expect( prtp.proxy.stats().server.nodecount ).to.equal( 1 )

    proxy.close()
    n.destroy()
  } )

  it( `create a remote (albeit local through the proxy) channel`, async function() {

    let proxy = prtp.proxy.listen()
    let n = await prtp.proxy.connect()

    await proxy.waitfornewconnection()
    expect( prtp.proxy.stats().server.nodecount ).to.equal( 1 )

    await prtp.openchannel()

    proxy.close()
    n.destroy()
  } )
} )
