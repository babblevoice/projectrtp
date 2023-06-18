
const expect = require( "chai" ).expect
const net = require( "net" )
const message = require( "../../lib/message.js" )
const prtp = require( "../../index.js" ).projectrtp
const { v4: uuidv4 } = require( "uuid" )

class mocknode {

  constructor() {
    this.ourstats = prtp.stats()
    this.messagehandlers = {}
    this.recevievedmessagecount = 0
    this.id = uuidv4()
    this.socks = []
  }

  /**
   * 
   * @returns { mocknode }
   */
  static create() {
    return new mocknode()
  }

  /**
   * 
   * @param { number } port 
   * @param { string } address 
   * @return { Promise }
   */
  async connect( port = 9002, address = "127.0.0.1" ) {

    const connectpromise = new Promise( resolve => this._newconnectresolve = resolve )
    const connection = net.createConnection( port, address, () => {

      this.connection = connection
      /* Pretend to be a node: our server will pass out new connections only after a
      stats message has been sent and it must have an instance id */
      const msg = {}
      msg.status = this.ourstats
      msg.status.instance = this.id
      this.connection.write( message.createmessage( msg ) )

      this._newconnectresolve()

      const mp = message.newstate()
      connection.on( "data", ( data ) => {
        message.parsemessage( mp, data, ( receivedmsg ) => {
          this.recevievedmessagecount++
          expect( receivedmsg ).to.have.property( "channel" ).that.is.a( "string" )
          expect( receivedmsg ).to.have.property( "id" ).that.is.a( "string" )
          this.messagehandlers[ receivedmsg.channel ]( receivedmsg, ( senddata ) => {
            senddata.status = this.ourstats
            connection.write( message.createmessage( senddata ) )
          } )
        } )
      } )
    } )

    await connectpromise
  }

  /**
   * Listen for connections to this mock node.
   * @param { number } [ port ] - default 9002, if 0 binnd to random
   * @return { Promise } 
   */
  async listen( port = 9002, host = "127.0.0.1" ) {
    let listenresolve
    const listenpromise = new Promise( ( r ) => listenresolve = r )
    this.port = port
    this.host = host
    this.server = net.createServer( ( connection ) => {
      connection.setKeepAlive( true )
      const mp = message.newstate()
      connection.on( "data", ( data ) => {
        message.parsemessage( mp, data, ( receivedmsg ) => {
          this.recevievedmessagecount++
          expect( receivedmsg ).to.have.property( "channel" ).that.is.a( "string" )
          expect( receivedmsg ).to.have.property( "id" ).that.is.a( "string" )
          this.messagehandlers[ receivedmsg.channel ]( receivedmsg, ( senddata ) => {
            senddata.status = this.ourstats
            connection.write( message.createmessage( senddata ) )
          } )
        } )
      } )
    } )

    if( 0 == port ) {
      this.server.listen( () => {
        // Port does exist?
        // @ts-ignore
        this.port =  this.server.address().port
        listenresolve( this.port )
      } )
    } else {
      this.server.listen( port, host, listenresolve )
    }
    
    return await listenpromise
  }

  /**
   * 
   * @param { string } event 
   * @param { function } cb 
   * @return { void }
   */
  setmessagehandler( event, cb ) {
    this.messagehandlers[ event ] = cb
  }

  /**
   * @return { Promise< void > }
   */
  async destroy() {
    if( this.connection ) {
      const p = new Promise( resolve => {
        this.connection.on( "close", resolve )
      } )

      this.connection.destroy()
      await p
    }

    if( this.server ) {
      const p = new Promise( resolve => {
        this.server.on( "close", resolve )
      } )
      this.server.close()
      await p
    }
  }

  /**
   * Will send a messag to the latst connection. If you expect multiple
   * connections for a node then use the senddata function passed into the
   * on data event.
   * @param { object } obj 
   * @return { void }
   */
  sendmessage( obj ) {
    obj.status = this.ourstats
    this.connection.write( message.createmessage( obj ) )
  }
}


module.exports = mocknode