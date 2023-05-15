
const prtp = require( "../../index" )
const node = require( "../../lib/node" )
const expect = require( "chai" ).expect

/**
 * Test file to run tests acting as a remote note. Starts a babble-rtp node in the background
 * before any tests begin then runs tests to open, play, polly etc.
 */
describe( "server connect interface", () => {

  before( async () => {
    const host = "127.0.0.1"
    const port = 9002
    await prtp.projectrtp.node.listen( host, port )
    prtp.projectrtp.server.addnode( { host, port} )
    
  } )

  after( () => {

    prtp.projectrtp.server.clearnodes()
    node.interface.destroy()
  } )

  it( "server connect and open channel", async function () {
    this.timeout( 6000 )
    this.slow( 3000 )

    const totalotherchannelcount = 100
    let chanclosecount = 0
    let allchannelsclosedresolve
    const allchannelsclosed = new Promise( resolve => allchannelsclosedresolve = resolve )
    const onclose = ( e ) => {
      if( "close" == e.action ) chanclosecount++
      if( totalotherchannelcount == chanclosecount ) allchannelsclosedresolve()
    }

    // A very short wav file
    prtp.projectrtp.tone.generate( "350+440*0.5:100", "/tmp/serverconnecttestwav.wav" )
    prtp.projectrtp.tone.generate( "350+440*0.5:100", "/tmp/otherserverconnecttestwav.wav" )

    const channels = []
    for( let i = 0; totalotherchannelcount > i; i++ ) {
      channels.push( await prtp.projectrtp.openchannel( onclose ) )
    }

    for( let i = 0; 3 > i; i++ ) {
      let done
      const finished = new Promise( resolve => done = resolve )
  
      const receivedmessages = []
      const chan = await prtp.projectrtp.openchannel( ( e ) => {
        receivedmessages.push( e )
        if( "play" == e.action && "end" == e.event ) chan.close()
        if( "close" == e.action ) done()
      } )
  
      chan.play( { "interupt":true, "files": [ { "wav": "/tmp/serverconnecttestwav.wav" }, { "wav": "/tmp/otherserverconnecttestwav.wav" } ] } )
  
      await finished

      //console.log(receivedmessages)
      expect( receivedmessages.length ).to.equal( 3 )
      expect( receivedmessages[ 0 ].action ).to.equal( "play" )
      expect( receivedmessages[ 0 ].event ).to.equal( "start" )
      expect( receivedmessages[ 0 ].reason ).to.equal( "new" )
      expect( receivedmessages[ 1 ].action ).to.equal( "play" )
      expect( receivedmessages[ 1 ].event ).to.equal( "end" )
      expect( receivedmessages[ 1 ].reason ).to.equal( "completed" )
      expect( receivedmessages[ 2 ].action ).to.equal( "close" )
      expect( receivedmessages[ 2 ].reason ).to.equal( "requested" )
    }

    for( const chan of channels ) {
      chan.close()
    }

    await allchannelsclosed
    expect( chanclosecount ).to.equal( totalotherchannelcount )
  } )

} )