
const expect = require( "chai" ).expect
const prtp = require( "../../index.js" ).projectrtp
const mocknode = require( "../mock/mocknode.js" )
const node = require( "../../lib/node.js" ).interface

/*
The tests in this file are to ensure what we send out over
our socket is in the correct format.
*/

let listenport = 45000

describe( "rtpproxy server", function() {

  function getnextport() {
    return listenport++
  }

  it( "start and stop and start listener", async function() {

    const ourport = getnextport()

    let p = await prtp.proxy.listen( undefined, "127.0.0.1", ourport )
    let n = new mocknode()

    await n.connect( ourport )
    n.destroy()
    await p.destroy()

    p = await prtp.proxy.listen( undefined, "127.0.0.1", ourport )
    n = new mocknode()

    await n.connect( ourport )
    n.destroy()
    await p.destroy()
  } )

  it( "check open json", async function() {
    /* set up our mock node object */
    const n = new mocknode()
    n.setmessagehandler( "open", ( onmsg ) => {
      n.sendmessage( {
        "action": "open",
        "id": onmsg.id,
        "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec152",
        "local": {
          "port": 10002,
          "address": "192.168.0.141"
        }
      } )
    } )

    let closereceived = false
    n.setmessagehandler( "close", ( msg ) => {

      n.sendmessage( {
        "action": "close",
        "id": msg.id,
        "uuid": msg.uuid
      } )

      closereceived = true
    } )

    const ourport = getnextport()
    const p = await prtp.proxy.listen( undefined, "127.0.0.1", ourport )
    await n.connect( ourport )
    const channel = await prtp.openchannel()

    expect( channel ).to.be.an( "object" )
    expect( channel.close ).to.be.an( "function" )
    expect( channel.mix ).to.be.an( "function" )
    expect( channel.unmix ).to.be.an( "function" )
    expect( channel.echo ).to.be.an( "function" )
    expect( channel.play ).to.be.an( "function" )
    expect( channel.record ).to.be.an( "function" )
    expect( channel.direction ).to.be.an( "function" )
    expect( channel.dtmf ).to.be.an( "function" )
    expect( channel.remote ).to.be.an( "function" )
    expect( channel.local ).to.have.property( "port" ).that.is.a( "number" )
    expect( channel.local ).to.have.property( "address" ).that.is.a( "string" )
    expect( channel.local.port ).to.equal( 10002 )
    expect( channel.local.address ).to.equal( "192.168.0.141" )
    expect( channel.uuid ).that.is.a( "string" )
    expect( channel.id ).that.is.a( "string" )

    await channel.close()

    n.destroy()
    p.destroy()

    expect( closereceived ).to.be.true
  } )

  it( "check echo", async function() {

    /* set up our mock node object */
    const n = new mocknode()
    n.setmessagehandler( "open", ( onmsg ) => {
      n.sendmessage( {
        "action": "open",
        "id": onmsg.id,
        "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec152",
        "channel": {
          "port": 10002,
          "address": "192.168.0.141"
        }
      } )
    } )

    let receivedecho = false
    n.setmessagehandler( "echo", () => {
      receivedecho = true
    } )

    n.setmessagehandler( "close", ( msg ) => {
      n.sendmessage( {
        "action": "close",
        "id": msg.id,
        "uuid": msg.uuid
      } )
    } )

    const ourport = getnextport()
    const p = await prtp.proxy.listen( undefined, "127.0.0.1", ourport )
    await n.connect( ourport )
    const channel = await prtp.openchannel()
    channel.echo()
    await channel.close()

    n.destroy()
    p.destroy()


    expect( receivedecho ).to.be.true
  } )

  it( "check dtmf", async function() {

    /* set up our mock node object */
    const n = new mocknode()
    n.setmessagehandler( "open", ( onmsg ) => {
      n.sendmessage( {
        "action": "open",
        "id": onmsg.id,
        "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec152",
        "channel": {
          "port": 10002,
          "address": "192.168.0.141"
        }
      } )
    } )

    let reveiveddtmf = false
    n.setmessagehandler( "dtmf", ( msg ) => {
      reveiveddtmf = true
      expect( msg ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "dtmf" )
      expect( msg ).to.have.property( "id" ).that.is.a( "string" )
      expect( msg ).to.have.property( "uuid" ).that.is.a( "string" )
      expect( msg ).to.have.property( "digits" ).that.is.a( "string" ).to.equal( "#123" )
    } )

    n.setmessagehandler( "close", ( msg ) => {
      n.sendmessage( {
        "action": "close",
        "id": msg.id,
        "uuid": msg.uuid
      } )
    } )

    const ourport = getnextport()
    const p = await prtp.proxy.listen( undefined, "127.0.0.1", ourport )
    await n.connect( ourport )
    const channel = await prtp.openchannel()
    channel.dtmf( "#123" )
    await channel.close()

    n.destroy()
    p.destroy()

    expect( reveiveddtmf ).to.be.true
  } )

  it( "check mix/unmix", async function() {

    /* set up our mock node object */
    const n = new mocknode()
    let uuidcount = 1
    n.setmessagehandler( "open", ( msg ) => {
      n.sendmessage( {
        "action": "open",
        "id": msg.id,
        "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec15" + uuidcount++,
        "channel": {
          "port": 10002,
          "address": "192.168.0.141"
        }
      } )
    } )

    let mxmsg = {}
    n.setmessagehandler( "mix", ( msg ) => mxmsg = msg )

    let unmxmsg = {}
    n.setmessagehandler( "unmix", ( msg ) => unmxmsg = msg )

    n.setmessagehandler( "close", ( msg ) => {
      n.sendmessage( {
        "action": "close",
        "id": msg.id,
        "uuid": msg.uuid
      } )
    } )

    const ourport = getnextport()
    const p = await prtp.proxy.listen( undefined, "127.0.0.1", ourport )
    await n.connect( ourport )
    const channela = await prtp.openchannel()
    const channelb = await prtp.openchannel()
    channela.mix( channelb )
    channela.unmix()

    await Promise.all( [
      channela.close(),
      channelb.close()
    ] )

    n.destroy()
    p.destroy()

    expect( mxmsg ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "mix" )
    expect( mxmsg ).to.have.property( "other" ).that.is.a( "object" )
    expect( mxmsg.other ).to.have.property( "id" ).that.is.a( "string" )
    expect( mxmsg.other ).to.have.property( "uuid" ).that.is.a( "string" )
    expect( mxmsg ).to.have.property( "id" ).that.is.a( "string" )
    expect( mxmsg ).to.have.property( "uuid" ).that.is.a( "string" )

    expect( unmxmsg ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "unmix" )
    expect( unmxmsg ).to.have.property( "id" ).that.is.a( "string" )
    expect( unmxmsg ).to.have.property( "uuid" ).that.is.a( "string" )

  } )

  it( "check node selection", async function() {

    const n = new mocknode()
    const n2 = new mocknode()
    let openedcount = 0

    n.setmessagehandler( "open", ( msg ) => {
      n.sendmessage( {
        "action": "open",
        "id": msg.id,
        "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec15" +  openedcount++,
        "local": {
          "port": 10002,
          "address": "192.168.0.141"
        },
        "status": n.ourstats
      } )
    } )
    
    n.setmessagehandler( "close", ( msg ) => {
      n.sendmessage( {
        "action": "close",
        "id": msg.id,
        "uuid": msg.uuid
      } )
    } )

    n2.setmessagehandler( "open", ( msg ) => {
      n2.sendmessage( {
        "action": "open",
        "id": msg.id,
        "uuid": "9dfc35d9-eafe-4d8b-8880-c48f528ec15" + openedcount++,
        "local": {
          "port": 10004,
          "address": "192.168.0.141"
        },
        "status": n2.ourstats
      } )

    } )

    n2.setmessagehandler( "close", ( msg ) => {
      n.sendmessage( {
        "action": "close",
        "id": msg.id,
        "uuid": msg.uuid
      } )
    } )

    const ourport = getnextport()
    const p = await prtp.proxy.listen( undefined, "127.0.0.1", ourport )
    await n.connect( ourport )
    await n2.connect( ourport )
    
    const channel1 = await prtp.openchannel()
    const channel2 = await prtp.openchannel()

    await Promise.all( [
      channel1.close(),
      channel2.close()
    ] )
    

    n.destroy()
    n2.destroy()
    await p.destroy()

    expect( openedcount ).to.equal( 2 )
  } )

  it( "check remote", async function() {

    /* set up our mock node object */
    const n = new mocknode()
    n.setmessagehandler( "open", ( msg ) => {
      n.sendmessage( {
        "action": "open",
        "id": msg.id,
        "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec152",
        "channel": {
          "port": 10002,
          "address": "192.168.0.141"
        }
      } )
    } )

    let remotereceived = false
    n.setmessagehandler( "remote", ( msg ) => {
      expect( msg ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "remote" )
      expect( msg ).to.have.property( "id" ).that.is.a( "string" )
      expect( msg ).to.have.property( "uuid" ).that.is.a( "string" )
      expect( msg ).to.have.property( "remote" ).that.is.a( "string" ).to.equal( "wouldbearemoteobject" )

      remotereceived = true
    } )

    n.setmessagehandler( "close", ( msg ) => {
      n.sendmessage( {
        "action": "close",
        "id": msg.id,
        "uuid": msg.uuid
      } )
    } )

    const ourport = getnextport()
    const p = await prtp.proxy.listen( undefined, "127.0.0.1", ourport )
    await n.connect( ourport )
    const channel = await prtp.openchannel()
    channel.remote( "wouldbearemoteobject" )

    await channel.close()
    n.destroy()
    p.destroy()

    expect( remotereceived ).to.be.true
  } )


  it( "check play/record", async function() {

    let done
    const completed = new Promise( ( r ) => { done = r } )

    /* set up our mock node object */
    const n = new mocknode()
    n.setmessagehandler( "open", ( msg ) => {
      n.sendmessage( {
        "action": "open",
        "id": msg.id,
        "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec152",
        "channel": {
          "port": 10002,
          "address": "192.168.0.141"
        }
      } )
    } )

    let playmsg
    n.setmessagehandler( "play", ( msg ) => {
      playmsg = msg

      n.sendmessage( {
        "action": "play",
        "id": msg.id,
        "uuid": msg.uuid
      } )

      setTimeout( () => channel.record( "wouldbearecordobject" ), 10 )
    } )

    let recmsg
    n.setmessagehandler( "record", ( msg ) => {
      recmsg = msg
      done()
    } )

    n.setmessagehandler( "close", ( msg ) => {
      n.sendmessage( {
        "action": "close",
        "id": msg.id,
        "uuid": msg.uuid
      } )
    } )

    const ourport = getnextport()
    const p = await prtp.proxy.listen( undefined, "127.0.0.1", ourport )
    await n.connect( ourport )
    const channel = await prtp.openchannel()
    channel.play( "wouldbeaplayobject" )

    await completed
    await channel.close()

    n.destroy()
    p.destroy()

    expect( recmsg ).to.be.an( "object" )
    expect( recmsg ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "record" )
    expect( recmsg ).to.have.property( "id" ).that.is.a( "string" )
    expect( recmsg ).to.have.property( "uuid" ).that.is.a( "string" )
    expect( recmsg ).to.have.property( "options" ).that.is.a( "string" ).to.equal( "wouldbearecordobject" )

    expect( playmsg ).to.be.an( "object" )
    expect( playmsg ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "play" )
    expect( playmsg ).to.have.property( "id" ).that.is.a( "string" )
    expect( playmsg ).to.have.property( "uuid" ).that.is.a( "string" )
    expect( playmsg ).to.have.property( "soup" ).that.is.a( "string" ).to.equal( "wouldbeaplayobject" )

    expect( channel.history ).to.be.an( "array" ).to.have.length( 7 )

  } )

  it( "check direction( { send, recv } )", async function() {

    let done
    const completed = new Promise( ( r ) => { done = r } )

    /* set up our mock node object */
    const n = new mocknode()
    n.setmessagehandler( "open", ( msg ) => {
      n.sendmessage( {
        "action": "open",
        "id": msg.id,
        "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec152",
        "local": {
          "port": 10002,
          "address": "192.168.0.141"
        }
      } )
    } )

    let directionmsg = {
      send: true,
      recv: true
    }

    n.setmessagehandler( "direction", ( msg ) => {
      directionmsg = msg.options

      n.sendmessage( {
        "action": "direction",
        "id": msg.id,
        "uuid": msg.uuid
      } )

      done()
    } )

    n.setmessagehandler( "close", ( msg ) => {
      n.sendmessage( {
        "action": "close",
        "id": msg.id,
        "uuid": msg.uuid
      } )
    } )

    const ourport = getnextport()
    const p = await prtp.proxy.listen( undefined, "127.0.0.1", ourport )
    await n.connect( ourport )
    const channel = await prtp.openchannel()
    channel.direction( { send: false, recv: false } )

    await completed
    await channel.close()

    n.destroy()
    p.destroy()

    expect( directionmsg.send ).to.be.false
    expect( directionmsg.recv ).to.be.false
  } )

  it( "Mocknode as listener server to test", async () => {

    let openreceived = false

    const ourport = getnextport()
    prtp.proxy.clearnodes()
    prtp.proxy.addnode( { host: "127.0.0.1", port: ourport } )
    const n = new mocknode()

    await n.listen( ourport )

    n.setmessagehandler( "open", ( msg, sendmessage ) => {
      openreceived = true
      sendmessage( {
        "action": "open",
        "id": msg.id,
        "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec152",
        "local": {
          "port": 10000,
          "address": "192.168.0.141"
        }
      } )
    } )

    n.setmessagehandler( "close", ( msg, sendmessage ) => {
      sendmessage( {
        "action": "close",
        "uuid": msg.uuid,
        "id": msg.id
      } )
    } )
    
    const chnl = await prtp.openchannel()
    expect( chnl ).to.have.property( "id" ).that.is.a( "string" )
    expect( chnl ).to.have.property( "uuid" ).that.is.a( "string" )
    expect( chnl ).to.have.property( "local" ).that.is.a( "object" )
    expect( chnl.local ).to.have.property( "port" ).that.is.a( "number" )
    expect( chnl.local ).to.have.property( "address" ).that.is.a( "string" )
    await chnl.close()
    
    n.destroy()
    
    expect( openreceived ).to.be.true

  } )

  it( "Two mock nodes listening, 2 openchannels on the same node close each in turn to ensure connection is maintained", async () => {
    
    let openreceived = false

    let ouruuid = 1
    const n1port = getnextport()
    const n2port = getnextport()

    prtp.proxy.clearnodes()
    prtp.proxy.addnode( { host: "127.0.0.1", port: n1port } )
    prtp.proxy.addnode( { host: "127.0.0.1", port: n2port } )

    const n = new mocknode()
    const n2 = new mocknode()

    await n.listen( n1port )
    await n2.listen( n2port )

    n.setmessagehandler( "open", ( msg, sendmessage ) => {
      openreceived = true
      sendmessage( {
        "action": "open",
        "id": msg.id,
        "uuid": ""+ouruuid++,
        "local": {
          "port": 10000,
          "address": "127.0.0.1"
        }
      } )
    } )

    n2.setmessagehandler( "open", ( msg, sendmessage ) => {
      openreceived = true
      sendmessage( {
        "action": "open",
        "id": msg.id,
        "uuid": ""+ouruuid++,
        "local": {
          "port": 10002,
          "address": "127.0.0.1"
        }
      } )
    } )


    n.setmessagehandler( "close", ( msg, sendmessage ) => {
      sendmessage( {
        "action": "close",
        "uuid": msg.uuid,
        "id": msg.id
      } )
    } )

    n2.setmessagehandler( "close", ( msg, sendmessage ) => {
      sendmessage( {
        "action": "close",
        "uuid": msg.uuid,
        "id": msg.id
      } )
    } )

    const chnl = await prtp.openchannel()
    const chnl2 = await chnl.openchannel()

    expect( chnl.connection ).to.be.equal( chnl2.connection )

    Promise.all( [
      chnl.close(),
      chnl2.close()
    ] )

    n.destroy()
    n2.destroy()
    expect( openreceived ).to.be.true
  } )

  it( "1 channel, node as listener server to test", async () => {

    const ourport = getnextport()
    prtp.server.clearnodes()
    prtp.server.addnode( { host: "127.0.0.1", port: ourport } )

    const ournode = node.create( prtp )

    const n = await ournode.listen( "127.0.0.1", ourport )
    const chnl = await prtp.openchannel()
    await chnl.close()
    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 100 ) } )
    n.destroy()
  } )

  it( "2 channels, node as listener server to test", async () => {

    const ourport = getnextport()
    prtp.server.clearnodes()
    prtp.server.addnode( { host: "127.0.0.1", port: ourport } )

    const ournode = node.create( prtp )

    const n = await ournode.listen( "127.0.0.1", ourport )
    const chnl = await prtp.openchannel()
    const chnl2 = await prtp.openchannel()
    await chnl.close()
    await chnl2.close()
    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 100 ) } )
    n.destroy()
  } )

  it( "2 channels, node as listener server to test - open a channel through another one", async () => {

    const ourport = getnextport()
    prtp.server.clearnodes()
    prtp.server.addnode( { host: "127.0.0.1", port: ourport } )

    const ournode = node.create( prtp )

    const n = await ournode.listen( "127.0.0.1", ourport )
    const chnl = await prtp.openchannel()
    const chnl2 = await chnl.openchannel()
    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 100 ) } )
    await chnl.close()
    await chnl2.close()
    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 100 ) } )
    n.destroy()
  } )

  it( "Ensure we receive middle messages", async() => {
    /*
      i.e. messages we receive inbetween an open and close
    */

    const ourport = getnextport()
    prtp.server.clearnodes()
    prtp.server.addnode( { host: "127.0.0.1", port: ourport } )

    const ournode = node.create( prtp )
    const n = await ournode.listen( "127.0.0.1", ourport )

    const recvedmsgs = []
    const chnl = await prtp.openchannel( ( e ) => {
      recvedmsgs.push( e )
    } )

    chnl.play( { "interupt":true, "files": [ { "wav": "test/interface/pcaps/180fa1ac-08e5-11ed-bd4d-02dba5b5aad6.wav" } ] } )

    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 100 ) } )

    await chnl.close()
    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 100 ) } )

    n.destroy()

    expect( recvedmsgs[ 0 ].action ).to.equal( "play" )
    expect( recvedmsgs[ 0 ].event ).to.equal( "start" )
    expect( recvedmsgs[ 1 ].action ).to.equal( "play" )
    expect( recvedmsgs[ 1 ].event ).to.equal( "end" )
    expect( recvedmsgs[ 2 ].action ).to.equal( "close" )
    

  } )
  it( "livestats over the node protocol", async function() {

    /* callmanager-side: poll a relay leg that lives on a remote node, the
       "registered but no media flowing" detector for video */
    const ourport = getnextport()
    prtp.server.clearnodes()
    const p = await prtp.proxy.listen( undefined, "127.0.0.1", ourport )
    const ournode = await prtp.node.connect( ourport, "127.0.0.1" )

    const events = []
    const chnl = await prtp.openchannel( { "relay": true }, ( e ) => events.push( e ) )
    expect( chnl.connection ).to.be.an( "object" ) /* proxied, not local */

    const [ s1, s2 ] = await Promise.all( [ chnl.livestats(), chnl.livestats() ] )
    for( const s of [ s1, s2 ] ) {
      expect( s.relay ).to.be.true
      expect( s.in ).to.include( { count: 0, accepted: 0, rtcp: 0, decryptfailed: 0, prekey: 0 } )
      expect( s.out ).to.include( { count: 0, dropped: 0 } )
    }
    /* polling is not a channel event */
    expect( events.filter( ( e ) => "livestats" === e.action ) ).to.have.lengthOf( 0 )
    expect( chnl.history.filter( ( m ) => "livestats" === m.channel || "livestats" === m.action ) ).to.have.lengthOf( 0 )

    await chnl.close()
    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 100 ) } )
    ournode.destroy()
    p.destroy()
  } )

  it( "relaylimits reach a relay leg opened on a remote node", async function() {

    /* the abuse limits are enforced in the node's native relay, so they
       must survive the node protocol - and a proxied leg opened without
       them must still get the defaults */
    const ourport = getnextport()
    prtp.server.clearnodes()
    const p = await prtp.proxy.listen( undefined, "127.0.0.1", ourport )
    const ournode = await prtp.node.connect( ourport, "127.0.0.1" )

    const limited = await prtp.openchannel( { "relay": true, "relaylimits": { "bitrate": 500000, "framerate": 15, "maxfs": 0 } } )
    const defaulted = await prtp.openchannel( { "relay": true } )
    expect( limited.connection ).to.be.an( "object" ) /* proxied, not local */

    const l = await limited.livestats()
    expect( l.relaylimits ).to.deep.equal( { "bitrate": 500000, "burst": 62500, "packetrate": 1000, "framerate": 15, "maxfs": 0 } )
    expect( l.in ).to.include( { "ratedropped": 0, "framedropped": 0, "oversizedropped": 0 } )
    const d = await defaulted.livestats()
    expect( d.relaylimits ).to.deep.equal( { "bitrate": 2000000, "burst": 250000, "packetrate": 1000, "framerate": 30, "maxfs": 3600 } )

    await limited.close()
    await defaulted.close()
    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 100 ) } )
    ournode.destroy()
    p.destroy()
  } )

  it( "livestats carries a relay leg's stream mapping over the node protocol", async function() {

    const dgram = require( "dgram" )
    const bind = () => new Promise( ( resolve ) => {
      const s = dgram.createSocket( "udp4" )
      s.bind( 0, "127.0.0.1", () => resolve( s ) )
    } )

    const ourport = getnextport()
    prtp.server.clearnodes()
    const p = await prtp.proxy.listen( undefined, "127.0.0.1", ourport )
    const ournode = await prtp.node.connect( ourport, "127.0.0.1" )

    const alice = await bind()
    const bob = await bind()
    const a = await prtp.openchannel( { "relay": true, "remote": { "address": "127.0.0.1", "port": alice.address().port, "codec": 96 } } )
    const b = await prtp.openchannel( { "relay": true, "remote": { "address": "127.0.0.1", "port": bob.address().port, "codec": 100 } } )
    expect( a.connection ).to.be.an( "object" ) /* proxied, not local */
    await a.mix( b )
    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 100 ) } )

    const rtp = Buffer.alloc( 112 )
    rtp[ 0 ] = 0x80
    rtp[ 1 ] = 96
    rtp.writeUInt32BE( 0xa11ce, 8 )
    alice.send( rtp, a.local.port, "127.0.0.1" )
    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 200 ) } )

    const s = await b.livestats()
    expect( s.streams ).to.deep.equal( [ { "source": a.uuid, "ssrc": b.local.ssrc, "sourcessrc": 0xa11ce, "pt": 100 } ] )
    expect( ( await a.livestats() ).streams ).to.deep.equal( [] )

    await a.close()
    await b.close()
    alice.close()
    bob.close()
    ournode.destroy()
    p.destroy()
  } )

  it( "livestats rejects when the node never answers", async function() {

    const n = new mocknode()
    n.setmessagehandler( "open", ( msg ) => {
      n.sendmessage( { "action": "open", "id": msg.id, "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec1ff", "channel": { "port": 10002, "address": "192.168.0.141" } } )
    } )
    let asked
    n.setmessagehandler( "livestats", ( msg ) => asked = msg )
    n.setmessagehandler( "close", ( msg ) => n.sendmessage( { "action": "close", "id": msg.id, "uuid": msg.uuid } ) )

    const ourport = getnextport()
    prtp.server.clearnodes()
    const p = await prtp.proxy.listen( undefined, "127.0.0.1", ourport )
    await n.connect( ourport )
    const chnl = await prtp.openchannel()

    let err
    try { await chnl.livestats( 200 ) } catch( e ) { err = e }
    expect( err ).to.be.an( "error" )
    expect( asked ).to.include( { "channel": "livestats", "id": chnl.id, "uuid": chnl.uuid } )

    await chnl.close()
    n.destroy()
    p.destroy()
  } )

  it( "concurrent livestats calls each get their own reply", async function() {

    /* every call used to register em.once( "livestats" ), so the first
       reply resolved all outstanding calls with the same answer */
    const n = new mocknode()
    n.setmessagehandler( "open", ( msg ) => {
      n.sendmessage( { "action": "open", "id": msg.id, "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec1ff", "channel": { "port": 10002, "address": "192.168.0.141" } } )
    } )
    const asked = []
    n.setmessagehandler( "livestats", ( msg ) => asked.push( msg ) )
    n.setmessagehandler( "close", ( msg ) => n.sendmessage( { "action": "close", "id": msg.id, "uuid": msg.uuid } ) )

    const ourport = getnextport()
    prtp.server.clearnodes()
    const p = await prtp.proxy.listen( undefined, "127.0.0.1", ourport )
    await n.connect( ourport )
    const chnl = await prtp.openchannel()

    const reply = ( req, count, echo = true ) => n.sendmessage( {
      "action": "livestats", "id": chnl.id, "uuid": chnl.uuid,
      ...( echo ? { "reqid": req.reqid } : {} ),
      "livestats": { "relay": true, "in": { count }, "out": { "count": 0 } } } )
    const waitasked = async ( k ) => { while( asked.length < k ) await new Promise( ( r ) => setTimeout( r, 5 ) ) }

    /* answered out of order: each caller still gets its own */
    const first = chnl.livestats()
    const second = chnl.livestats()
    await waitasked( 2 )
    expect( asked[ 0 ].reqid ).to.not.equal( asked[ 1 ].reqid )
    reply( asked[ 1 ], 2 )
    reply( asked[ 0 ], 1 )
    expect( ( await first ).in.count ).to.equal( 1 )
    expect( ( await second ).in.count ).to.equal( 2 )

    /* a late reply to a call that timed out is not handed to the next one */
    let err
    try { await chnl.livestats( 50 ) } catch( e ) { err = e }
    expect( err ).to.be.an( "error" )
    const third = chnl.livestats()
    await waitasked( 4 )
    reply( asked[ 2 ], 3 )
    reply( asked[ 3 ], 4 )
    expect( ( await third ).in.count ).to.equal( 4 )

    /* a node that predates reqid: replies arrive in request order */
    const fourth = chnl.livestats()
    const fifth = chnl.livestats()
    await waitasked( 6 )
    reply( asked[ 4 ], 5, false )
    reply( asked[ 5 ], 6, false )
    expect( ( await fourth ).in.count ).to.equal( 5 )
    expect( ( await fifth ).in.count ).to.equal( 6 )

    await chnl.close()
    n.destroy()
    p.destroy()
  } )

  it( "Ensure connection stays open with 0 channel in listen mode", async () => {

    const ourport = getnextport()
    prtp.server.clearnodes()
    const p = await prtp.proxy.listen( undefined, "127.0.0.1", ourport )
    const ournode = await prtp.node.connect( ourport, "127.0.0.1" )

    const chnl = await prtp.openchannel()
    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 100 ) } )
    await chnl.close()
    const chnl2 = await prtp.openchannel()
    await chnl2.close()
    await new Promise( ( resolve ) => { setTimeout( () => resolve(), 100 ) } )
    ournode.destroy()
    p.destroy()
  } )
} )
