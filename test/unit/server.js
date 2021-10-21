
const expect = require( "chai" ).expect
const prtp = require( "../../index.js" ).projectrtp
const mocknode = require( "./mocknode.js" )

describe( "node", function() {
  it( `node check open json`, async function() {

    /* set up our mock node object */
    let n = new mocknode()
    n.setmessagehandler( "open", ( onmsg ) => {
      n.sendmessage( {
          "action": "open",
          "id": onmsg.id,
          "channel": {
            "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec152",
            "port": 10002,
            "ip": "192.168.0.141"
            }
          } )
    } )

    let p = await prtp.proxy.listen( 45000, "127.0.0.1" )
    n.connect()
    await p.waitfornewconnection()
    let channel = await prtp.openchannel()

    expect( channel ).to.be.an( "object" )
    expect( channel.close ).to.be.an( "function" )
    expect( channel.mix ).to.be.an( "function" )
    expect( channel.unmix ).to.be.an( "function" )
    expect( channel.echo ).to.be.an( "function" )
    expect( channel.play ).to.be.an( "function" )
    expect( channel.record ).to.be.an( "function" )
    expect( channel.direction ).to.be.an( "function" )
    expect( channel.dtmf ).to.be.an( "function" )
    expect( channel.target ).to.be.an( "function" )
    expect( channel.local ).to.have.property( "port" ).that.is.a( "number" )
    expect( channel.local ).to.have.property( "ip" ).that.is.a( "string" )
    expect( channel.local.port ).to.equal( 10002 )
    expect( channel.local.ip ).to.equal( "192.168.0.141" )
    expect( channel.uuid ).that.is.a( "string" )
    expect( channel.id ).that.is.a( "string" )

    channel.close()

    p.close()
    n.destroy()
  } )
} )
