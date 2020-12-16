

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

#include "controlclient.h"
#include "projectrtpchannel.h"

extern rtpchannels dormantchannels;
extern std::string publicaddress;

activertpchannels activechannels;

/*!md
# parserequest
As it says...

Leave this function at the top of the file as this is the definition of the json structure we receive (and send).
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

        p->open( id, u );

        if( body.has_key( "target" ) )
        {
          /* Set the target */
          JSON::Object target = JSON::as_object( body[ "target" ] );
          short port = JSON::as_int64( target[ "port" ] );
          p->target( JSON::as_string( target[ "ip" ] ), port );
        }

        if( body.has_key( "audio" ) )
        {
          JSON::Object audio = JSON::as_object( body[ "audio" ] );
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

            p->audio( ourcodeclist );
          }
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
      else if( "close" == action )
      {
        std::string channel = JSON::as_string( body[ "uuid" ] );
        activertpchannels::iterator chan = activechannels.find( channel );
        if ( activechannels.end() != chan )
        {
          chan->second->close();
          dormantchannels.push_back( chan->second );
          activechannels.erase( chan );
        }

        JSON::Object v;
        v[ "action" ] = "close";
        v[ "uuid" ] = channel;
        this->sendmessage( v );
      }
      else if( "play" == action )
      {
        std::string channel = JSON::as_string( body[ "uuid" ] );
        std::string soup = JSON::as_string( body[ "soup" ] );
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
        v[ "error" ] = "Unknown channel";
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
          v[ "error" ] = "Unknown channel";
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
    this->sendmessage( v );
  }
}

void controlclient::sendmessage( JSON::Object &v )
{
  JSON::Object s;
  JSON::Object c;
  c[ "active" ] = ( JSON::Integer ) activechannels.size();
  c[ "available" ] = ( JSON::Integer ) dormantchannels.size();
  s[ "channels" ] = c;

  v[ "status" ] = s;

  stringptr t = stringptr( new std::string( JSON::to_string( v ) ) );
  this->dowrite( t );
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

  this->json = NULL;
  this->jsonreservedlengthed = 0;
  this->jsonamountread = 0;
}

controlclient::~controlclient()
{
  if( NULL != this->json ) delete this->json;
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
      if( NULL != this->json ) delete this->json;

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
  std::cout << "Sent: " << *msg << std::endl;
  bool writeinprogress = !this->outboundmessages.empty();

  this->outboundmessages.push_back( stringptr( msg ) );
  if ( !writeinprogress )
  {
    char *outheaderbufptr = &this->outheaderbuf[ 0 ];
    *outheaderbufptr = 0x33;
    outheaderbufptr++;
    *( ( uint16_t * ) outheaderbufptr ) = 0;
    outheaderbufptr += 2;
    *( ( uint16_t * ) outheaderbufptr ) = htons( msg->size() );

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
          boost::asio::buffer( outheaderbuf, CONTROLHEADERLENGTH ),
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
