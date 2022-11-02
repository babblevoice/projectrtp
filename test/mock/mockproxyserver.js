
const net = require( "net" )
const message = require( "../../lib/message.js" )

let listenport = 55000
module.exports = class {

  constructor() {
    listenport++
    this.socks = []
    this.messagehandlers = {}
  }

  async connect( port = 9002, address = "127.0.0.1" ) {

    console.log("connecting", port, address)
    port = 9002
    let newconnectresolve
    let connectpromise = new Promise( r => newconnectresolve = r )
    this.connection = net.createConnection( port, address )
    this.connection.on( "connect", () => newconnectresolve() )

    await connectpromise
  }

  listen() {
    this.port = listenport
    this.server = net.createServer( this._onnewconnection.bind( this ) )

    this.server.listen( listenport, "127.0.0.1" )
    this.server.on( "listening", this._onsocketlistening.bind( this ) )
    this.server.on( "close", this._oncloseconnection.bind( this ) )
  }

  _onsocketdata( data ) {
    message.parsemessage( this.mp, data, ( receivedmsg ) => {
      this.recevievedmessagecount++
      expect( receivedmsg ).to.have.property( "channel" ).that.is.a( "string" )
      expect( receivedmsg ).to.have.property( "id" ).that.is.a( "string" )
      this.messagehandlers[ receivedmsg.channel ]( receivedmsg )
    } )
  }

  setmessagehandler( event, cb ) {
    this.messagehandlers[ event ] = cb
  }

  close() {
    return new Promise( resolve => {
      this.closingpromiseresolve = resolve
      for( let sock in this.socks ) {
        try{
          this.socks[ sock ].destroy()
        } catch( e ){ console.log( e ) }
      }

      this.server.close()

      if( undefined !== this._onnewconnectionpromise ) {
        this._onnewconnectionpromise()
        delete this._onnewconnectionpromise
      }
    } )
  }

  onnewconnection( cb ) {
    this.onnewconn = cb
  }

  _onnewconnection( sock ) {
    this.socks.push( sock )

    if( undefined !== this.onnewconn ) {
      this.onnewconn( sock )
    }
  }

  _onsocketlistening() {
  }

  _oncloseconnection() {
    if( undefined !== this.closingpromiseresolve ) {
      this.closingpromiseresolve()
      delete this.closingpromiseresolve
    }
  }

}
