
const net = require( "net" )
const { v4: uuidv4 } = require( "uuid" )
const message = require( "./message.js" )

const instance = uuidv4()
const channels = new Map()

const channelmap = {
  "close": ( chan, msg ) => chan.close(),
  "target": ( chan, msg ) => chan.target( msg.target ),
  "mix": ( chan, msg ) => chan.mix( msg.other ),
  "unmix": ( chan, msg ) => chan.unmix(),
  "dtmf": ( chan, msg ) => chan.dtmf( msg.digits ),
  "echo": ( chan, msg ) => chan.echo(),
  "play": ( chan, msg ) => chan.play( msg.soup ),
  "record": ( chan, msg ) => chan.record( msg.options ),
  "direction": ( chan, msg ) => chan.direction( msg.options )
}

/**
@summary An RTP node. We are a remote RTP node which waits for
intruction from our server.
@memberof proxy
@hideconstructor
*/
class rtpnode {
  constructor( prtp ) {
    this.prtp = prtp
    this.instance = uuidv4()
    this.messagestate = message.newstate()
  }

  /**
  @summary Connect to a server.
  @param {number} port
  @param {string} host
  @return {Promise<rtpnode>} - Promise which resolves to an rtpnode
  */
  connect( port, host ) {
    this.host = host
    this.port = port

    return new Promise( resolve => {
      this._onsocketreadypromiseresolve = resolve
      this.connection = net.createConnection( this.port, this.host )
      this.connection.setKeepAlive( true )

      this.connection.on( "connect", this._onsocketconnect.bind( this ) )
      this.connection.on( "data", this._onsocketdata.bind( this ) )
    } )
  }

  /**
  @summary Destroy this node
  */
  destroy() {
    this.connection.destroy()
  }

  _onsocketconnect() {
    this._sendmessage( {} )
    this._onsocketreadypromiseresolve( this )
  }

  _sendmessage( msg ) {

    msg.status = this.prtp.stats()
    msg.status.instance = instance
    this.connection.write( message.createmessage( msg ) )
  }

  _onsocketdata( data ) {
    message.parsemessage( this.messagestate, data, async ( msg ) => {
      return ( await this._openchannel( msg ) || this._updatechannel( msg ) )
    } )
  }

  async _openchannel( msg ) {
    if( "open" !== msg.channel ) return false
    msg.forcelocal = true
    let channelidentifiers = {
      "id": msg.id,
      "uuid": uuidv4()
    }
    let chan = await this.prtp.openchannel( msg, ( x ) => {
      if( "close" === x.action ) channels.delete( channelidentifiers.uuid )
      this.connection.write( message.createmessage( { ...x, ...channelidentifiers } ) )
    } )

    channels.set( channelidentifiers.uuid, chan )
    this.connection.write( message.createmessage( { ...chan, ...channelidentifiers } ) )

    return true
  }
  _updatechannel( msg ) {

    if( undefined === msg.channel ) return false
    if( undefined === msg.uuid ) return false

    let chan = channels.get( msg.uuid )
    if( undefined == chan ) return false

    if( msg.channel in channelmap ) {
      channelmap[ msg.channel ]( chan, msg )
    } else {
      let channelidentifiers = {
        "id": msg.id,
        "uuid": msg.uuid
      }
      this.connection.write( message.createmessage( { ...{ "error": "Unknown method" }, ...channelidentifiers } ) )
    }

    return true
  }
}

module.exports.connect = ( prtp, port = 9002, host ) => {
  let n = new rtpnode( prtp )
  return n.connect( port, host )
}
