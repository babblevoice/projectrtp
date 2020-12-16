

#ifndef PROJECTRTPCONTROLSERVER_H
#define PROJECTRTPCONTROLSERVER_H

#include <boost/asio.hpp>
#include <boost/asio/ip/tcp.hpp>

#include <deque>

#include "json.hpp"
typedef std::shared_ptr< std::string > stringptr;
typedef std::deque< stringptr > waitingmessages;

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

  void write( const stringptr msg );
  void close();
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
  void parserequest( void );
  void sendmessage( JSON::Object &v );

private:

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
  boost::asio::io_context& iocontext;
  boost::asio::ip::tcp::socket socket;
  boost::asio::ip::tcp::resolver resolver;
  boost::asio::steady_timer retrytimer;
  boost::asio::ip::tcp::resolver::results_type endpoints;
};


#endif /* PROJECTRTPCONTROLSERVER_H */
