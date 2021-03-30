

#include <cstdlib>
#include <iostream>
#include <memory>
#include <utility>
#include <boost/bind/bind.hpp>
#include <boost/asio.hpp>

#include <boost/uuid/uuid.hpp>
#include <boost/uuid/uuid_generators.hpp>
#include <boost/uuid/uuid_io.hpp>
#include <boost/lexical_cast.hpp>

#include <exception>
#include <stdexcept>

#include "controlclient.h"
#include "projectrtpchannel.h"

extern rtpchannels dormantchannels;
extern std::string publicaddress;

activertpchannels activechannels;

/* Called by other threads to clean up a channel that has closed */
void controlclient::channelclosed( std::string &uuid )
{
  boost::asio::post( this->iocontext,
        boost::bind( &controlclient::dochannelclosed, this, stringptr( new std::string( uuid ) ) ) );
}

/* Called by the single threaded ioconext after channelclosed has been called */
void controlclient::dochannelclosed( stringptr uuid )
{
  activertpchannels::iterator chan = activechannels.find( *uuid );
  if ( activechannels.end() != chan )
  {
    dormantchannels.push_back( chan->second );
    activechannels.erase( chan );
  }
}

static void parsetarget( projectrtpchannel::pointer p, JSON::Object &target )
{
  /* Set the target */
  short port = JSON::as_int64( target[ "port" ] );
  p->target( JSON::as_string( target[ "ip" ] ), port );

  if( target.has_key( "audio" ) )
  {
    JSON::Object audio = JSON::as_object( target[ "audio" ] );
    if( audio.has_key( "payloads" ) )
    {
      JSON::Array payloads = JSON::as_array( audio[ "payloads" ] );

      /* this is the CODECs we have been asked to send to the remote endpoint */
      projectrtpchannel::codeclist ourcodeclist;

      for( JSON::Array::values_t::iterator it = payloads.values.begin();
            it != payloads.values.end();
            it++ )
      {
        ourcodeclist.push_back( JSON::as_int64( *it ) );
      }

      if( !p->audio( ourcodeclist ) )
      {
        std::cerr << "No suitable audio CODEC provided" << std::endl;
      }
    }
    else
    {
      std::cerr << "No audio.payloads provided" << std::endl;
    }
  }
  else
  {
    std::cerr << "No audio provided" << std::endl;
  }
}

/*!md
# parserequest
As it says...

Leave this function near the top of the file as this is the definition of the json structure we receive (and send).
*/
void controlclient::parserequest( void )
{
  try
  {
    std::cout << "Received: " << this->json << std::endl;
    JSON::Object body = JSON::as_object( JSON::parse( this->json ) );

    if( body.has_key( "channel" ) )
    {
      std::string action = JSON::as_string( body[ "channel" ] );
      if( "open" == action )
      {
        std::cout << "Opening channel" << std::endl;
        if( dormantchannels.size() == 0 )
        {
          /* report error */
          std::cerr << "We have been asked to open a channel but have none left to give." << std::endl;
          return;
        }

        /* our unique identifier */
        boost::uuids::uuid uuid = boost::uuids::random_generator()();
        std::string u = boost::lexical_cast<std::string>( uuid );
        projectrtpchannel::pointer p = *dormantchannels.begin();
        dormantchannels.pop_front();
        activechannels[ u ] = p;

        std::string id;
        if( body.has_key( "id" ) ) {
          id = JSON::as_string( body[ "id" ] );
        }

        p->open( id, u, shared_from_this() );

        if( body.has_key( "target" ) )
        {
          JSON::Object target = JSON::as_object( body[ "target" ] );
          parsetarget( p, target );
        }

        JSON::Object v;
        JSON::Object c;
        v[ "id" ] = id;
        v[ "action" ] = "open";
        c[ "uuid" ] = u;
        c[ "port" ] = ( JSON::Integer ) p->getport();
        c[ "ip" ] = publicaddress;

        v[ "channel" ] = c;
        this->sendmessage( v );
      }
      else if( "target" == action )
      {
        std::string channel = JSON::as_string( body[ "uuid" ] );
        activertpchannels::iterator chan = activechannels.find( channel );
        if ( activechannels.end() != chan )
        {
          JSON::Object target = JSON::as_object( body[ "target" ] );
          parsetarget( chan->second, target );
        }
      }
      else if( "direction" == action )
      {
        std::string channel = JSON::as_string( body[ "uuid" ] );
        activertpchannels::iterator chan = activechannels.find( channel );
        if ( activechannels.end() != chan )
        {
          chan->second->direction( JSON::as_boolean( body[ "send" ] ) == JSON::Bool( true ), JSON::as_boolean( body[ "recv" ] ) == JSON::Bool( true ) );
          JSON::Object v;
          v[ "action" ] = "direction";
          v[ "uuid" ] = channel;
          this->sendmessage( v );
        }
        else
        {
          JSON::Object v;
          v[ "error" ] = "No such channel";
          this->sendmessage( v );
        }
      }
      else if( "rfc2833" == action )
      {
        std::string channel = JSON::as_string( body[ "uuid" ] );
        activertpchannels::iterator chan = activechannels.find( channel );
        if ( activechannels.end() != chan )
        {
          unsigned short pt = JSON::as_int64( body[ "pt" ] );
          chan->second->rfc2833( pt );

          JSON::Object v;
          v[ "action" ] = "rfc2833";
          v[ "uuid" ] = channel;
          this->sendmessage( v );
        }
        else
        {
          JSON::Object v;
          v[ "error" ] = "No such channel";
          this->sendmessage( v );
        }
      }
      else if( "close" == action )
      {
        std::string channel = JSON::as_string( body[ "uuid" ] );
        activertpchannels::iterator chan = activechannels.find( channel );
        if ( activechannels.end() != chan )
        {
          chan->second->close();
        }
        /* our channel will send close on completion */
      }
      else if( "record" == action )
      {
        std::string channel = JSON::as_string( body[ "uuid" ] );
        std::string filename = JSON::as_string( body[ "file" ] );
        channelrecorder::pointer p = channelrecorder::create( filename );

        p->uuid = boost::lexical_cast<std::string>( boost::uuids::random_generator()() );

        if( body.has_key( "maxduration" ) )
        {
          p->maxduration = JSON::as_int64( body[ "maxduration" ] );
        }

        if( body.has_key( "minduration" ) )
        {
          p->minduration = JSON::as_int64( body[ "minduration" ] );
        }

        if( body.has_key( "poweraverageduration" ) )
        {
          p->poweraverageduration = JSON::as_int64( body[ "poweraverageduration" ] );
        }

        if( body.has_key( "poweraverageduration" ) )
        {
          p->poweraverageduration = JSON::as_int64( body[ "poweraverageduration" ] );
        }

        if( body.has_key( "finishbelowpower" ) )
        {
          p->finishbelowpower = JSON::as_int64( body[ "finishbelowpower" ] );
        }

        if( body.has_key( "startabovepower" ) )
        {
          p->startabovepower = JSON::as_int64( body[ "startabovepower" ] );
        }

        activertpchannels::iterator chan = activechannels.find( channel );
        if ( activechannels.end() != chan )
        {
          chan->second->record( p );
        }
      }
      else if( "play" == action )
      {
        std::string channel = JSON::as_string( body[ "uuid" ] );
        std::string soup = JSON::to_string( body[ "soup" ] );
        activertpchannels::iterator chan = activechannels.find( channel );
        if ( activechannels.end() != chan )
        {
          chan->second->play( stringptr( new std::string( soup ) ) );
        }

        JSON::Object v;
        v[ "action" ] = "play";
        v[ "uuid" ] = channel;
        this->sendmessage( v );
      }
      else if( "echo" == action )
      {
        std::string channel = JSON::as_string( body[ "uuid" ] );
        activertpchannels::iterator chan = activechannels.find( channel );
        if ( activechannels.end() != chan )
        {
          chan->second->echo();
        }

        JSON::Object v;
        v[ "action" ] = "echo";
        v[ "uuid" ] = channel;
        this->sendmessage( v );
      }
      else if( "mix" == action )
      {
        JSON::Array channels = JSON::as_array( body[ "uuid" ] );
        activertpchannels::iterator chan1 = activechannels.find( JSON::as_string( channels[ 0 ] ) );
        activertpchannels::iterator chan2 = activechannels.find( JSON::as_string( channels[ 1 ] ) );
        if ( activechannels.end() != chan1 && activechannels.end() != chan2 )
        {
          if( chan1->second->mix( chan2->second ) )
          {
            JSON::Object v;
            v[ "action" ] = "mix";
            v[ "uuid" ] = body[ "uuid" ];
            this->sendmessage( v );
            return;
          }
        }

        JSON::Object v;
        v[ "action" ] = "unmix";
        v[ "error" ] = "Unknown mix channel";
        this->sendmessage( v );
      }
      else if( "unmix" == action )
      {
        std::string channel = JSON::as_string( body[ "uuid" ] );
        activertpchannels::iterator chan = activechannels.find( channel );
        if ( activechannels.end() != chan )
        {
          chan->second->unmix();

          JSON::Object v;
          v[ "action" ] = "unmix";
          v[ "uuid" ] = channel;
          this->sendmessage( v );
        }
        else
        {
          JSON::Object v;
          v[ "action" ] = "unmix";
          v[ "error" ] = "Unknown unmix channel";
          this->sendmessage( v );
        }
      }
    }
  }
  catch( boost::bad_get &e )
  {
    JSON::Object v;
    v[ "error" ] = "Bad request";
    this->sendmessage( v );
  }
  catch( ... )
  {
    JSON::Object v;
    v[ "error" ] = "Unknown error occured";

    try
    {
      auto eptr = std::current_exception();
      if ( eptr )
      {
          std::rethrow_exception( eptr );
      }
    }
    catch( const std::exception& e )
    {
        v[ "error" ] = e.what();
    }

    this->sendmessage( v );
  }
}

void controlclient::sendmessage( JSON::Object &v )
{
  stringptr t = stringptr( new std::string( JSON::to_string( v ) ) );
  this->write( t );
}

/*!md
# create

*/
controlclient::pointer controlclient::create( boost::asio::io_context &iocontext, std::string &host )
{
  return pointer( new controlclient( iocontext, host ) );
}

controlclient::controlclient( boost::asio::io_context& io_context, std::string &host )
    : controlhost( host ),
      iocontext( io_context ),
      socket( io_context ),
      resolver( io_context ),
      retrytimer( io_context )
{
  this->endpoints = resolver.resolve( controlhost.c_str(), "9002" );

  boost::asio::async_connect( this->socket, this->endpoints,
      boost::bind( &controlclient::handleconnect, this,
                    boost::asio::placeholders::error ) );

  this->json = nullptr;
  this->jsonreservedlengthed = 0;
  this->jsonamountread = 0;

  boost::uuids::uuid uuid = boost::uuids::random_generator()();
  this->uuid = boost::lexical_cast<std::string>( uuid );
}

controlclient::~controlclient()
{
  if( nullptr != this->json ) delete[] this->json;
  this->json = nullptr;
}

void controlclient::write( const stringptr msg )
{
  boost::asio::post( this->iocontext,
        boost::bind( &controlclient::dowrite, this, msg ) );
}

void controlclient::close()
{
  boost::asio::post( this->iocontext,
                      boost::bind( &controlclient::doclose, this ) );
}

void controlclient::reconnect( const boost::system::error_code& error )
{
  if (!error)
  {
    this->endpoints = resolver.resolve( controlhost.c_str(), "9002" );

    boost::asio::async_connect( this->socket, this->endpoints,
        boost::bind( &controlclient::handleconnect, this,
                      boost::asio::placeholders::error ) );
  }
}

void controlclient::tryreconnect( void )
{
  this->retrytimer.expires_from_now( boost::asio::chrono::seconds( 5 ) );
  this->retrytimer.async_wait( boost::bind( &controlclient::reconnect, this,
                boost::asio::placeholders::error ) );
}

void controlclient::handleconnect(const boost::system::error_code& error)
{
  if (!error)
  {
    /* Send an empty object so stats get added */
    JSON::Object v;
    v[ "action" ] = "connected";
    this->sendmessage( v );

    boost::asio::async_read( this->socket,
        boost::asio::buffer( this->headerbuff, CONTROLHEADERLENGTH ),
        boost::bind(&controlclient::handlereadheader, this,
          boost::asio::placeholders::error));
  }
  else
  {
    this->tryreconnect();
  }
}

void controlclient::handlereadheader( const boost::system::error_code& error )
{
  if( !error )
  {
    char *bufptr = &this->headerbuff[ 0 ];
    this->header.magik = *bufptr;
    bufptr++;
    this->header.version = ntohs( *( (uint16_t * ) bufptr ) );
    bufptr += 2;
    this->header.length = ntohs( *( (uint16_t * ) bufptr ) );

    if( 0x33 != this->header.magik ||
        0 == this->header.length )
    {
      std::cerr << "Bad magik or zero length - barfing" << std::endl;
      this->doclose();
      return;
    }

    this->jsonamountread = 0;

    if( this->jsonreservedlengthed < this->header.length )
    {
      if( nullptr != this->json ) delete[] this->json;

      this->json = new char[ this->header.length + 1 ];
      this->jsonreservedlengthed = this->header.length;
    }

    this->json[ this->header.length ] = 0;
    boost::asio::async_read( this->socket,
        boost::asio::buffer( this->json, this->header.length ),
        boost::bind( &controlclient::handlereadbody, this,
          boost::asio::placeholders::error ) );
  }
  else
  {
    doclose();
  }
}

void controlclient::handlereadbody(const boost::system::error_code& error)
{
  if ( !error )
  {
    parserequest();

    boost::asio::async_read( socket,
        boost::asio::buffer( this->headerbuff, CONTROLHEADERLENGTH ),
        boost::bind( &controlclient::handlereadheader, this,
          boost::asio::placeholders::error ) );
  }
  else
  {
    doclose();
  }
}

/* First write a header */
void controlclient::dowrite( stringptr msg )
{
  /* add other stats */
  JSON::Object msgbody = JSON::as_object( JSON::parse( *msg ) );
  JSON::Object s;
  JSON::Object c;
  msgbody[ "instance" ] = this->uuid;
  c[ "active" ] = ( JSON::Integer ) activechannels.size();
  c[ "available" ] = ( JSON::Integer ) dormantchannels.size();
  s[ "channels" ] = c;

  msgbody[ "status" ] = s;

  stringptr togo = stringptr( new std::string( JSON::to_string( msgbody ) ) );
  std::cout << "Sent: " << *togo << std::endl;

  bool writeinprogress = !this->outboundmessages.empty();

  this->outboundmessages.push_back( togo );
  if ( !writeinprogress )
  {
    char *outheaderbufptr = &this->outheaderbuf[ 0 ];
    *outheaderbufptr = 0x33;
    outheaderbufptr++;
    *( ( uint16_t * ) outheaderbufptr ) = 0;
    outheaderbufptr += 2;
    *( ( uint16_t * ) outheaderbufptr ) = htons( togo->size() );

    boost::asio::async_write( this->socket,
        boost::asio::buffer( this->outheaderbuf, CONTROLHEADERLENGTH ),
        boost::bind( &controlclient::handlewriteheader, this,
                      boost::asio::placeholders::error ) );
  }
}

void controlclient::handlewriteheader( const boost::system::error_code& error )
{
  if (!error)
  {
    if ( !this->outboundmessages.empty() )
    {
      /* It should always get here */
      boost::asio::async_write( this->socket,
          boost::asio::buffer( this->outboundmessages.front()->c_str(),
                                this->outboundmessages.front()->size() ),
          boost::bind( &controlclient::handlewritebody, this,
                        boost::asio::placeholders::error) );
    }
  }
  else
  {
    this->doclose();
  }
}

void controlclient::handlewritebody( const boost::system::error_code& error )
{
  if (!error)
  {
    this->outboundmessages.pop_front();

    if ( !this->outboundmessages.empty() )
    {
      char *outheaderbuf = &this->outheaderbuf[ 0 ];
      *outheaderbuf = 0x33;
      outheaderbuf++;
      *( ( uint16_t * ) outheaderbuf ) = 0;
      outheaderbuf += 2;
      *( ( uint16_t * ) outheaderbuf ) = htons( this->outboundmessages.front()->size() );

      boost::asio::async_write( this->socket,
          boost::asio::buffer( this->outheaderbuf, CONTROLHEADERLENGTH ),
          boost::bind( &controlclient::handlewriteheader, this,
                        boost::asio::placeholders::error ) );
    }
  }
  else
  {
    this->doclose();
  }
}

void controlclient::doclose()
{
  this->socket.close();
  this->tryreconnect();
}
