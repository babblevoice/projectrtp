
const expect = require( "chai" ).expect
const prtp = require( "../../index.js" ).projectrtp
const mocknode = require( "../mock/mocknode.js" )

/*
The tests in this file are to ensure what we send out over
our socket is in the correct format.
*/

let listenport = 45000

describe( "rtpproxy server", function() {

  afterEach( function() {
    /* when in listen mode a server doesn't appear ot release the bind
    immediatly, so in order to move on to the next test - use a different port */
    listenport++
  } )

  it( `start and stop and start listener`, async function() {

    let p = await prtp.proxy.listen( listenport, "127.0.0.1" )
    let n = new mocknode()

    n.connect( listenport )
    await p.waitfornewconnection()
    n.destroy()
    await p.close()

    p = await prtp.proxy.listen( listenport, "127.0.0.1" )
    n = new mocknode()

    n.connect( listenport )
    await p.waitfornewconnection()
    n.destroy()
    await p.close()
  } )

  it( `check open json`, async function() {
    /* set up our mock node object */
    let n = new mocknode()
    n.setmessagehandler( "open", ( onmsg ) => {
      n.sendmessage( {
          "action": "open",
          "id": onmsg.id,
          "channel": {
            "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec152",
            "port": 10002,
            "address": "192.168.0.141"
            }
          } )
    } )

    let closereceived = false
    n.setmessagehandler( "close", ( onmsg ) => {
      closereceived = true
      n.destroy()
      p.close()
    } )

    let p = await prtp.proxy.listen( listenport, "127.0.0.1" )
    n.connect( listenport )
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
    expect( channel.local ).to.have.property( "address" ).that.is.a( "string" )
    expect( channel.local.port ).to.equal( 10002 )
    expect( channel.local.address ).to.equal( "192.168.0.141" )
    expect( channel.uuid ).that.is.a( "string" )
    expect( channel.id ).that.is.a( "string" )

    channel.close()

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 10 ) } )

    expect( closereceived ).to.be.true
  } )

  it( `check echo`, async function() {

    /* set up our mock node object */
    let n = new mocknode()
    n.setmessagehandler( "open", ( onmsg ) => {
      n.sendmessage( {
          "action": "open",
          "id": onmsg.id,
          "channel": {
            "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec152",
            "port": 10002,
            "address": "192.168.0.141"
            }
          } )
    } )

    let receivedecho = false
    n.setmessagehandler( "echo", ( onmsg ) => {
      receivedecho = true
    } )

    let closereceived = false
    n.setmessagehandler( "close", ( onmsg ) => {
      n.destroy()
      p.close()
      closereceived = true
    } )

    let p = await prtp.proxy.listen( listenport, "127.0.0.1" )
    n.connect( listenport )
    await p.waitfornewconnection()
    let channel = await prtp.openchannel()
    channel.echo()
    channel.close()

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 10 ) } )

    expect( receivedecho ).to.be.true
    expect( closereceived ).to.be.true

  } )

  it( `check dtmf`, async function() {

    /* set up our mock node object */
    let n = new mocknode()
    n.setmessagehandler( "open", ( onmsg ) => {
      n.sendmessage( {
          "action": "open",
          "id": onmsg.id,
          "channel": {
            "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec152",
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

    let closereceived = false
    n.setmessagehandler( "close", ( onmsg ) => {
      n.destroy()
      p.close()
      closereceived = true
    } )

    let p = await prtp.proxy.listen( listenport, "127.0.0.1" )
    n.connect( listenport )
    await p.waitfornewconnection()
    let channel = await prtp.openchannel()
    channel.dtmf( "#123" )
    channel.close()

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 10 ) } )

    expect( closereceived ).to.be.true
    expect( reveiveddtmf ).to.be.true
  } )

  it( `check mix/unmix`, async function() {

    /* set up our mock node object */
    let n = new mocknode()
    n.setmessagehandler( "open", ( msg ) => {
      n.sendmessage( {
          "action": "open",
          "id": msg.id,
          "channel": {
            "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec152",
            "port": 10002,
            "address": "192.168.0.141"
            }
          } )
    } )

    let mixreceived = false
    let unmixreceived = false
    n.setmessagehandler( "mix", ( msg ) => {
      expect( msg ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "mix" )
      expect( msg ).to.have.property( "other" ).that.is.a( "string" ).to.equal( "otheruuid" )
      expect( msg ).to.have.property( "id" ).that.is.a( "string" )
      expect( msg ).to.have.property( "uuid" ).that.is.a( "string" )
      mixreceived = true
    } )

    n.setmessagehandler( "unmix", ( msg ) => {
      expect( msg ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "unmix" )
      expect( msg ).to.have.property( "id" ).that.is.a( "string" )
      expect( msg ).to.have.property( "uuid" ).that.is.a( "string" )
      unmixreceived = true
    } )

    let closereceived = false
    n.setmessagehandler( "close", ( msg ) => {
      n.destroy()
      p.close()
      closereceived = true
    } )

    let p = await prtp.proxy.listen( listenport, "127.0.0.1" )
    n.connect( listenport )
    await p.waitfornewconnection()
    let channel = await prtp.openchannel()
    channel.mix( "otheruuid" )
    channel.unmix()

    channel.close()

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 10 ) } )

    expect( mixreceived ).to.be.true
    expect( unmixreceived ).to.be.true

    expect( closereceived ).to.be.true

  } )

  it( `check target`, async function() {

    /* set up our mock node object */
    let n = new mocknode()
    n.setmessagehandler( "open", ( msg ) => {
      n.sendmessage( {
          "action": "open",
          "id": msg.id,
          "channel": {
            "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec152",
            "port": 10002,
            "address": "192.168.0.141"
            }
          } )
    } )

    let targetreceived = false
    n.setmessagehandler( "target", ( msg ) => {
      expect( msg ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "target" )
      expect( msg ).to.have.property( "id" ).that.is.a( "string" )
      expect( msg ).to.have.property( "uuid" ).that.is.a( "string" )
      expect( msg ).to.have.property( "target" ).that.is.a( "string" ).to.equal( "wouldbeatargetobject" )

      targetreceived = true
    } )

    let closereceived = false
    n.setmessagehandler( "close", ( msg ) => {
      n.destroy()
      p.close()
      closereceived = true
    } )

    let p = await prtp.proxy.listen( listenport, "127.0.0.1" )
    n.connect( listenport )
    await p.waitfornewconnection()
    let channel = await prtp.openchannel()
    channel.target( "wouldbeatargetobject" )

    channel.close()
    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 10 ) } )

    expect( targetreceived ).to.be.true
    expect( closereceived ).to.be.true

  } )


  it( `check play/record`, async function() {

    /* set up our mock node object */
    let n = new mocknode()
    n.setmessagehandler( "open", ( msg ) => {
      n.sendmessage( {
          "action": "open",
          "id": msg.id,
          "channel": {
            "uuid": "7dfc35d9-eafe-4d8b-8880-c48f528ec152",
            "port": 10002,
            "address": "192.168.0.141"
            }
          } )
    } )

    let playreceived = false
    let recordreceived = false
    n.setmessagehandler( "play", ( msg ) => {
      expect( msg ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "play" )
      expect( msg ).to.have.property( "id" ).that.is.a( "string" )
      expect( msg ).to.have.property( "uuid" ).that.is.a( "string" )
      expect( msg ).to.have.property( "soup" ).that.is.a( "string" ).to.equal( "wouldbeaplayobject" )

      playreceived = true
    } )

    n.setmessagehandler( "record", ( msg ) => {
      expect( msg ).to.have.property( "channel" ).that.is.a( "string" ).to.equal( "record" )
      expect( msg ).to.have.property( "id" ).that.is.a( "string" )
      expect( msg ).to.have.property( "uuid" ).that.is.a( "string" )
      expect( msg ).to.have.property( "options" ).that.is.a( "string" ).to.equal( "wouldbearecordobject" )

      recordreceived = true
    } )

    let closereceived = false
    n.setmessagehandler( "close", ( msg ) => {
      n.destroy()
      p.close()
      closereceived = true
    } )

    let p = await prtp.proxy.listen( listenport, "127.0.0.1" )
    n.connect( listenport )
    await p.waitfornewconnection()
    let channel = await prtp.openchannel()
    channel.play( "wouldbeaplayobject" )
    channel.record( "wouldbearecordobject" )

    channel.close()

    await new Promise( ( resolve, reject ) => { setTimeout( () => resolve(), 10 ) } )

    expect( playreceived ).to.be.true
    expect( recordreceived ).to.be.true
  } )
} )
