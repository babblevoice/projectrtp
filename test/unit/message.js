
const expect = require( "chai" ).expect

const m = require( "../../lib/message.js" )

describe( "message", function() {

  it( `simple test parse message`, async function() {
    let ms = m.newstate()

    /* This will also dictate our msg header - so increase
    above 8 bits in length */
    let tosend = {
      "test": "object",
      "testnumber": 3
    }

    let msg = JSON.stringify( tosend )

    let msgheader = Buffer.from( [ 0x33, 0x00, 0x00, 0x00, msg.length ] )
    let msgbuffer = Buffer.from( msg )

    m.parsemessage( ms, msgheader, ( msg ) => {
      expect( false ).to.be.true
    } )

    let received = false
    m.parsemessage( ms, msgbuffer, ( receivedmsg ) => {
      received = true
      expect( receivedmsg ).to.deep.equal( tosend )
    } )

    expect( received ).to.be.true
  } )

  it( `simple test break message`, async function() {
    let ms = m.newstate()

    /* This will also dictate our msg header - so increase
    above 8 bits in length */
    let tosend = {
      "test": "object",
      "testnumber": 3
    }

    let msg = JSON.stringify( tosend )

    let msgheader = Buffer.from( [ 0x33, 0x00, 0x00, 0x00, msg.length ] )
    let msgbuffer = Buffer.concat( [ msgheader, Buffer.from( msg ) ] )

    m.parsemessage( ms, msgbuffer.slice( 0, 10 ), ( msg ) => {
      expect( false ).to.be.true
    } )

    let received = false
    m.parsemessage( ms, msgbuffer.slice( 10 ), ( receivedmsg ) => {
      received = true
      expect( receivedmsg ).to.deep.equal( tosend )
    } )

    expect( received ).to.be.true
  } )

  it( `simple test break header message`, async function() {
    let ms = m.newstate()

    /* This will also dictate our msg header - so increase
    above 8 bits in length */
    let tosend = {
      "test": "object",
      "testnumber": 3
    }

    let msg = JSON.stringify( tosend )

    let msgheader = Buffer.from( [ 0x33, 0x00, 0x00, 0x00, msg.length ] )
    let msgbuffer = Buffer.concat( [ msgheader, Buffer.from( msg ) ] )

    m.parsemessage( ms, msgbuffer.slice( 0, 3 ), ( msg ) => {
      expect( false ).to.be.true
    } )

    let received = false
    m.parsemessage( ms, msgbuffer.slice( 3 ), ( receivedmsg ) => {
      received = true
      expect( receivedmsg ).to.deep.equal( tosend )
    } )

    expect( received ).to.be.true
  } )

  it( `simple test all at once message`, async function() {
    let ms = m.newstate()

    /* This will also dictate our msg header - so increase
    above 8 bits in length */
    let tosend = {
      "test": "object",
      "testnumber": 3
    }

    let msg = JSON.stringify( tosend )

    let msgheader = Buffer.from( [ 0x33, 0x00, 0x00, 0x00, msg.length ] )
    let msgbuffer = Buffer.concat( [ msgheader, Buffer.from( msg ) ] )

    let received = false
    m.parsemessage( ms, msgbuffer, ( receivedmsg ) => {
      received = true
      expect( receivedmsg ).to.deep.equal( tosend )
    } )

    expect( received ).to.be.true
  } )

  it( `send 2 all at once message`, async function() {
    let ms = m.newstate()

    /* This will also dictate our msg header - so increase
    above 8 bits in length */
    let tosend = {
      "test": "object",
      "testnumber": 3
    }

    let msgbuffer = m.createmessage( tosend )
    msgbuffer = Buffer.concat( [ msgbuffer, msgbuffer ] )

    let received = 0
    m.parsemessage( ms, msgbuffer, ( receivedmsg ) => {
      received++
      expect( receivedmsg ).to.deep.equal( tosend )
    } )

    expect( received ).to.equal( 2 )
  } )

  it( `simple create and parse test all at once message`, async function() {
    let ms = m.newstate()

    /* This will also dictate our msg header - so increase
    above 8 bits in length */
    let tosend = {
      "test": "object",
      "testnumber": 3
    }

    let msgbuffer = m.createmessage( tosend )

    let received = false
    m.parsemessage( ms, msgbuffer, ( receivedmsg ) => {
      received = true
      expect( receivedmsg ).to.deep.equal( tosend )
    } )

    expect( received ).to.be.true
  } )

  it( `send large packet > 255`, async function() {
    let ms = m.newstate()

    /* This will also dictate our msg header - so increase
    above 8 bits in length */
    let tosend = {
      "test": "object",
      "testdata": '#'.repeat( 500 )
    }

    let msgbuffer = m.createmessage( tosend )

    let received = false
    m.parsemessage( ms, msgbuffer, ( receivedmsg ) => {
      received = true
      expect( receivedmsg ).to.deep.equal( tosend )
    } )

    expect( received ).to.be.true
  } )
} )
