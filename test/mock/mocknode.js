
const expect = require( "chai" ).expect
const net = require( "net" )
const message = require( "../../lib/message.js" )
const prtp = require( "../../index.js" ).projectrtp
const { v4: uuidv4 } = require( "uuid" )

module.exports = class {
  constructor() {
    this.mp = message.newstate()
    this.ourstats = prtp.stats()
    this.messagehandlers = {}
    this.recevievedmessagecount = 0
  }



  connect( port = 9002, address = "127.0.0.1" ) {
    this.connection = net.createConnection( port, address )
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
    msg.status.instance = uuidv4()
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
