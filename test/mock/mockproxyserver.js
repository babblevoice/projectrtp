
const net = require( "net" )
const message = require( "../../lib/message.js" )

let listenport = 55000
module.exports = class {

  /** 
   * @type { function }
   * @private
   */
  closingpromiseresolve

  /**
   * @type { function }
   * @private
   */
  onnewconn

  constructor() {

    this.recevievedmessagecount = 0
    listenport++
    this.socks = []
    this.messagehandlers = {}
    this.mp = message.newstate()
  }

  /**
   * 
   * @param { number } port 
   * @param { string } address 
   * @returns { Promise< object > }
   */
  async connect( port = 9002, address = "127.0.0.1" ) {
    let newconnectresolve
    const connectpromise = new Promise( r => newconnectresolve = r )
    this.connection = net.createConnection( port, address )
    this.connection.on( "connect", () => newconnectresolve( this ) )
    this.connection.on( "data", this._onsocketdata.bind( this ) )

    await connectpromise
  }

  /**
   * Listens on selected port
   * @returns { Promise< object > }
   */
  listen() {
    let listening
    const listenpromise = new Promise( r => listening = r )

    this.port = listenport
    this.server = net.createServer( this._onnewconnection.bind( this ) )
    this.server.listen( listenport, "127.0.0.1" )
    this.server.on( "listening", () => {
      listening( this )
    } )
    this.server.on( "close", this._oncloseconnection.bind( this ) )

    return listenpromise
  }

  /**
   * @private
   * @param { Buffer } data 
   */
  _onsocketdata( data ) {
    message.parsemessage( this.mp, data, ( receivedmsg ) => {
      this.recevievedmessagecount++
      this.messagehandlers[ receivedmsg.action ]( receivedmsg )
    } )
  }

  /**
   * 
   * @param { string } event 
   * @param { function } cb 
   */
  setmessagehandler( event, cb ) {
    this.messagehandlers[ event ] = cb
  }

  /**
   * 
   * @returns { Promise }
   */
  close() {
    return new Promise( resolve => {
      this.closingpromiseresolve = resolve
      for( const sock in this.socks ) {
        try{
          this.socks[ sock ].destroy()
        } catch( e ){ console.log( e ) }
      }

      this.server.close()

      if( undefined !== this.closingpromiseresolve ) {
        this.closingpromiseresolve()
        delete this.closingpromiseresolve
      }
    } )
  }

  /**
   * 
   * @param { function } cb 
   * @returns { void }
   */
  onnewconnection( cb ) {
    this.onnewconn = cb
  }

  /**
   * @private
   * @param { net.Socket } sock 
   */
  _onnewconnection( sock ) {
    this.socks.push( sock )

    if( undefined !== this.onnewconn ) {
      this.onnewconn( sock )
    }
  }

  /**
   * @private
   * @return { void }
   */
  _oncloseconnection() {
    if( undefined !== this.closingpromiseresolve ) {
      this.closingpromiseresolve()
      delete this.closingpromiseresolve
    }
  }

  /**
   * 
   * @param { number } port 
   * @param { string } address
   * @return { Promise }
   */
  async openchannel( port, address ) {
    await this.connect( port, address )
    this.connection.write(
      message.createmessage( {
        "id": "54",
        "channel": "open"
      } ) )
  }
}
