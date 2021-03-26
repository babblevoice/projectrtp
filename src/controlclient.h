

#ifndef PROJECTRTPCONTROLSERVER_H
#define PROJECTRTPCONTROLSERVER_H

#include <boost/asio.hpp>
#include <boost/asio/ip/tcp.hpp>

#include <deque>

#include "globals.h"
#include "json.hpp"

typedef std::deque< stringptr > waitingmessages;
typedef std::shared_ptr< JSON::Object > jsonptr;

class controlheader
{
public:
  char magik;
  uint16_t version;
  uint16_t length;
};

#define CONTROLHEADERLENGTH 5 /* bytes */

class controlclient : public std::enable_shared_from_this< controlclient >
{
public:
  controlclient( boost::asio::io_context& io_context, std::string &host );
  ~controlclient();

  typedef std::shared_ptr< controlclient > pointer;
  static pointer create( boost::asio::io_context &iocontext, std::string &host );

  void write( const stringptr msg );
  void sendmessage( JSON::Object &v );
  void close();

  void channelclosed( std::string &uuid );

private:

  void handleconnect( const boost::system::error_code& error );
  void handlereadheader( const boost::system::error_code& error );
  void handlereadbody( const boost::system::error_code& error );
  void dowrite( stringptr msg );
  void handlewriteheader( const boost::system::error_code& error );
  void handlewritebody( const boost::system::error_code& error );
  void reconnect( const boost::system::error_code& error );
  void tryreconnect( void );
  void doclose();
  void dochannelclosed( stringptr uuid );
  void parserequest( void );

  /* inbound */
  char headerbuff[ CONTROLHEADERLENGTH ];
  controlheader header;
  char *json;
  int jsonreservedlengthed;
  int jsonamountread;

  /* outboud */
  char outheaderbuf[ CONTROLHEADERLENGTH ];
  waitingmessages outboundmessages;


  std::string controlhost;
  std::string uuid; /* instance uuid - this is generated when starting the server so a client can identify us */
  boost::asio::io_context& iocontext;
  boost::asio::ip::tcp::socket socket;
  boost::asio::ip::tcp::resolver resolver;
  boost::asio::steady_timer retrytimer;
  boost::asio::ip::tcp::resolver::results_type endpoints;
};


#endif /* PROJECTRTPCONTROLSERVER_H */
