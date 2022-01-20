/*
TODO
More of our interface requires testing. Enough to ensure all data is passed
over our proxy correctly. The correctness of a channel is tested in channel
tests.
*/

const expect = require( "chai" ).expect
const mockserver = require( "../mock/mockproxyserver.js" )
const prtp = require( "../../index.js" ).projectrtp
const message = require( "../../lib/message.js" )

describe( "rtpproxy node", function() {

  it( `connect to server`, async function() {

    let ourstate = message.newstate()

    await new Promise( async ( completed ) => {
      let mock = new mockserver()
      mock.listen()
      mock.onnewconnection( ( sock ) => {
        sock.on( "data", async ( data ) => {

          message.parsemessage( ourstate, data, async ( msg ) => {

            expect( msg ).to.have.property( "status" ).that.is.a( "object" )
            expect( msg.status ).to.have.property( "workercount" ).that.is.a( "number" )
            expect( msg.status ).to.have.property( "instance" ).that.is.a( "string" )
            expect( msg.status ).to.have.property( "channel" ).that.is.a( "object" )
            expect( msg.status.channel ).to.have.property( "available" ).that.is.a( "number" )
            expect( msg.status.channel ).to.have.property( "current" ).that.is.a( "number" )

            ournode.destroy()
            await mock.close()
            completed()
          } )
        } )
      } )

      let ournode = await prtp.proxy.connect( mock.port )
    } )
  } )

  it( `connect to server then drop the re-go`, async function() {

    this.timeout( 2000 )
    this.slow( 1500 )

    let ourstate = message.newstate()

    await new Promise( async ( completed ) => {
      let mock = new mockserver()
      mock.listen()
      mock.onnewconnection( ( sock ) => {
        sock.on( "data", async ( data ) => {

          message.parsemessage( ourstate, data, async ( msg ) => {

            expect( msg ).to.have.property( "status" ).that.is.a( "object" )
            expect( msg.status ).to.have.property( "workercount" ).that.is.a( "number" )
            expect( msg.status ).to.have.property( "instance" ).that.is.a( "string" )
            expect( msg.status ).to.have.property( "channel" ).that.is.a( "object" )
            expect( msg.status.channel ).to.have.property( "available" ).that.is.a( "number" )
            expect( msg.status.channel ).to.have.property( "current" ).that.is.a( "number" )

            /* drop the connection */
            for( let sock in mock.socks ) {
              try{
                mock.socks[ sock ].destroy()
              } catch( e ){ console.log( e ) }
            }

            if( 1 === connectcount ) {
              ournode.destroy()
              await mock.close()
              completed()
            }
            connectcount++
          } )
        } )
      } )

      let connectcount = 0
      let ournode = await prtp.proxy.connect( mock.port )
    } )
  } )

  it( `connect and open channel`, async function() {

    await new Promise( async ( completed ) => {
      let ourstate = message.newstate()
      let mock = new mockserver()
      mock.listen()
      let state = "begin"
      mock.onnewconnection( ( sock ) => {
        sock.on( "data", async ( data ) => {
          message.parsemessage( ourstate, data, async ( msg ) => {

            switch( state ) {
              case "begin": {
                /* We should receive our status after connecting */
                expect( msg ).to.have.property( "status" ).that.is.a( "object" )
                expect( msg.status ).to.have.property( "workercount" ).that.is.a( "number" )
                expect( msg.status ).to.have.property( "instance" ).that.is.a( "string" )
                expect( msg.status ).to.have.property( "channel" ).that.is.a( "object" )
                expect( msg.status.channel ).to.have.property( "available" ).that.is.a( "number" )
                expect( msg.status.channel ).to.have.property( "current" ).that.is.a( "number" )
                state = "open"
                sock.write(
                  message.createmessage( {
                    "id": "54",
                    "channel": "open"
                  } ) )
                return
              }
              case "open": {
                /* a confirmed open message */
                expect( msg ).to.have.property( "id" ).that.is.a( "string" )
                expect( msg ).to.have.property( "uuid" ).that.is.a( "string" )
                expect( msg ).to.have.property( "local" ).that.is.a( "object" )
                expect( msg.local ).to.have.property( "port" ).that.is.a( "number" )
                expect( msg.local ).to.have.property( "address" ).that.is.a( "string" )
                expect( msg.local ).to.have.property( "dtls" ).that.is.a( "object" )
                expect( msg.local.dtls ).to.have.property( "fingerprint" ).that.is.a( "string" )
                expect( msg.local.dtls ).to.have.property( "enabled" ).that.is.a( "boolean" )

                state = "close"
                sock.write(
                  message.createmessage( {
                    "id": msg.id,
                    "channel": "close",
                    "uuid": msg.uuid
                  } ) )
                return
              }
              case "close": {
                /* The fullness of this object is tested elsewhere - but confirm it is a close object we receive */
                expect( msg ).to.have.property( "action" ).that.is.a( "string" ).to.be.equal( "close" )
                expect( msg ).to.have.property( "id" ).that.is.a( "string" )
                expect( msg ).to.have.property( "uuid" ).that.is.a( "string" )
                expect( msg ).to.have.property( "stats" ).that.is.a( "object" )
                expect( msg.stats ).to.have.property( "in" ).that.is.a( "object" )
                expect( msg.stats ).to.have.property( "out" ).that.is.a( "object" )
                expect( msg.stats ).to.have.property( "tick" ).that.is.a( "object" )

                ournode.destroy()
                await mock.close()
                completed()
                return
              }
            }
          } )
        } )
      } )
      let ournode = await prtp.proxy.connect( mock.port )
    } )
  } )

  it( `bad method on node`, async function() {

    await new Promise( async ( completed ) => {
      let ourstate = message.newstate()
      let mock = new mockserver()
      mock.listen()
      let state = "begin"
      mock.onnewconnection( ( sock ) => {
        sock.on( "data", async ( data ) => {
          message.parsemessage( ourstate, data, async ( msg ) => {

            switch( state ) {
              case "begin": {
                state = "open"
                sock.write(
                  message.createmessage( {
                    "id": "54",
                    "channel": "open"
                  } ) )
                return
              }
              case "open": {
                state = "close"
                sock.write(
                  message.createmessage( {
                    "id": msg.id,
                    "channel": "blah",
                    "uuid": msg.uuid
                  } ) )
                return
              }
              case "close": {
                expect( msg ).to.have.property( "id" ).that.is.a( "string" )
                expect( msg ).to.have.property( "uuid" ).that.is.a( "string" )
                expect( msg ).to.have.property( "error" ).that.is.a( "string" )

                state = "closed"
                sock.write(
                  message.createmessage( {
                    "id": msg.id,
                    "channel": "close",
                    "uuid": msg.uuid
                  } ) )
                return
              }
              case "closed": {
                /* The fullness of this object is tested elsewhere - but confirm it is a close object we receive */
                expect( msg ).to.have.property( "action" ).that.is.a( "string" ).to.be.equal( "close" )
                ournode.destroy()
                await mock.close()
                completed()
                return
              }
            }
          } )
        } )
      } )
      let ournode = await prtp.proxy.connect( mock.port )
    } )
  } )

  it( `echo channel`, async function() {

    await new Promise( async ( completed ) => {
      let ourstate = message.newstate()
      let mock = new mockserver()
      mock.listen()
      let state = "begin"
      mock.onnewconnection( ( sock ) => {
        sock.on( "data", async ( data ) => {
          message.parsemessage( ourstate, data, async ( msg ) => {

            switch( state ) {
              case "begin": {
                state = "open"
                sock.write(
                  message.createmessage( {
                    "id": "54",
                    "channel": "open"
                  } ) )
                return
              }
              case "open": {
                state = "close"
                sock.write(
                  message.createmessage( {
                    "id": msg.id,
                    "channel": "echo",
                    "uuid": msg.uuid
                  } ) )
                sock.write(
                  message.createmessage( {
                    "id": msg.id,
                    "channel": "close",
                    "uuid": msg.uuid
                  } ) )
                return
              }
              case "close": {
                /* The fullness of this object is tested elsewhere - but confirm it is a close object we receive */
                expect( msg ).to.have.property( "action" ).that.is.a( "string" ).to.be.equal( "close" )
                ournode.destroy()
                await mock.close()
                completed()
                return
              }
            }
          } )
        } )
      } )
      let ournode = await prtp.proxy.connect( mock.port )
    } )
  } )

  it( `echo with onpre and onpost`, async function() {

    await new Promise( async ( completed ) => {
      let ourstate = message.newstate()
      let mock = new mockserver()
      mock.listen()
      let state = "begin"
      mock.onnewconnection( ( sock ) => {
        sock.on( "data", async ( data ) => {
          message.parsemessage( ourstate, data, async ( msg ) => {
            switch( state ) {
              case "begin": {
                state = "open"
                sock.write(
                  message.createmessage( {
                    "id": "54",
                    "channel": "open"
                  } ) )
                return
              }
              case "open": {
                state = "close"
                sock.write(
                  message.createmessage( {
                    "id": msg.id,
                    "channel": "echo",
                    "uuid": msg.uuid
                  } ) )
                sock.write(
                  message.createmessage( {
                    "id": msg.id,
                    "channel": "close",
                    "uuid": msg.uuid
                  } ) )
                return
              }
              case "close": {
                /* The fullness of this object is tested elsewhere - but confirm it is a close object we receive */
                expect( msg ).to.have.property( "action" ).that.is.a( "string" ).to.be.equal( "close" )
                ournode.destroy()
                await mock.close()

                expect( onprecount ).to.equal( 3 )
                expect( onpostcount ).to.equal( 2 )
                completed()
                return
              }
            }
          } )
        } )
      } )
      let ournode = await prtp.proxy.connect( mock.port )

      let onprecount = 0
      const expectedpremessages = [
        { "channel": "open" },
        { "channel": "echo" },
        { "channel": "close" }
      ]

      /*
      The onpre and onpost methods registers callbacks which can be used
      by external framework before we action a command or sendback
      a responce.
      */
      ournode.onpre( ( msg, cb ) => {
        expect( msg ).to.deep.include( expectedpremessages[ onprecount ] )
        onprecount++
        cb( msg )
      } )

      let onpostcount = 0
      const expectedpostmessages = [
        { "action": "open" },
        { "action": "close" }
      ]
      ournode.onpost( ( msg, cb ) => {
        expect( msg ).to.deep.include( expectedpostmessages[ onpostcount ] )
        onpostcount++
        cb( msg )
      } )
    } )
  } )
} )
