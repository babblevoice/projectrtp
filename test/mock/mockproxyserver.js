
const net = require( "net" )
const message = require( "../../lib/message.js" )

let listenport = 55000
module.exports = class {

  constructor() {
    listenport++
    this.socks = []
  }

  listen() {
    this.port = listenport
    this.server = net.createServer( this._onnewconnection.bind( this ) )

    this.server.listen( listenport, "127.0.0.1" )
    this.server.on( "listening", this._onsocketlistening.bind( this ) )
    this.server.on( "close", this._oncloseconnection.bind( this ) )
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
