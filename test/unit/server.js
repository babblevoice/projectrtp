
const expect = require( "chai" ).expect
const prtp = require( "../../index.js" ).projectrtp
const message = require( "../../lib/message.js" )

const net = require( "net" )

class mocknode {
  constructor() {
    this.mp = message.newstate()
    this.ourstats = prtp.stats()
    this.messagehandlers = {}
    this.recevievedmessagecount = 0
  }

  connect() {
    this.connection = net.createConnection( 45000, "127.0.0.1" )
    this.connection.on( "connect", this._onsocketconnect.bind( this ) )
    this.connection.on( "data", this._onsocketdata.bind( this ) )
  }

  setmessagehandler( event, cb ) {
    this.messagehandlers[ event ] = cb
  }

  destroy() {
    this.connection.destroy()
  }

  _onsocketconnect() {

    /* Pretend to be a node: our server will pass out new connections only after a
    stats message has been sent and it must have an instance id */
    let msg = {}
    msg.status = this.ourstats
    msg.instance = "1"
    this.connection.write( message.createmessage( msg ) )
  }

  _onsocketdata( data ) {
    message.parsemessage( this.mp, data, ( receivedmsg ) => {
      this.recevievedmessagecount++
      expect( receivedmsg ).to.have.property( "channel" ).that.is.a( "string" )
      expect( receivedmsg ).to.have.property( "id" ).that.is.a( "string" )
      this.messagehandlers[ receivedmsg.channel ]( receivedmsg )
    } )
  }

  sendmessage( obj ) {
    obj.status = this.ourstats
    this.connection.write( message.createmessage( obj ) )
  }

  destroy() {
    this.connection.destroy()
  }
}

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

/*
switch( receivedmessagecount ) {
  case 0: {
    expect( receivedmsg.status.channel ).to.have.property( "available" ).that.is.a( "number" )
    expect( receivedmsg.status.channel ).to.have.property( "current" ).that.is.a( "number" )
    expect( receivedmsg.status ).to.have.property( "workercount" ).that.is.a( "number" )
    expect( receivedmsg.status ).to.have.property( "instance" ).that.is.a( "string" )

    break
  }
}

*/
