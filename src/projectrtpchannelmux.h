

#ifndef PROJECTRTPCHANNELMUX_H
#define PROJECTRTPCHANNELMUX_H

#include <boost/asio.hpp>
#include <memory>
#include <atomic>

#include "projectrtpchannel.h"

/*
# projectchannelmux

Generate our own tick. If we get up to multiple channels we don't want all to have a timer
firing - we want only one to mix all. A channel will maintain its own timer (when needed)
for things like playing a sound file (on single channels) or echo.
*/

typedef std::shared_ptr< projectrtpchannel > projectrtpchannelptr;

typedef std::list< projectrtpchannelptr > projectchanptrlist;

class projectchannelmux:
  public std::enable_shared_from_this< projectchannelmux > {

public:
  projectchannelmux( boost::asio::io_context &iocontext );
  ~projectchannelmux();
  typedef std::shared_ptr< projectchannelmux > pointer;
  static pointer create( boost::asio::io_context &iocontext );

  void handletick( const boost::system::error_code& error );
  void postrtpdata( projectrtpchannelptr srcchan, projectrtpchannelptr dstchan, rtppacket *src );
  inline size_t size() { return this->channels.size(); }
  void addchannel( projectrtpchannelptr chan );
  void addchannels( projectrtpchannelptr chana, projectrtpchannelptr chanb );
  void go( void );

private:

  void checkfornewmixes( void );
  void mix2( void );
  void mixall( void );

  void setnexttick( void );

  static bool channelremoverequested( const projectrtpchannelptr& value );

  projectchanptrlist channels;

  boost::asio::io_context &iocontext;
  boost::asio::steady_timer tick;
  std::chrono::high_resolution_clock::time_point nexttick;

  projectchanptrlist newchannels;
  std::atomic_bool newchannelslock;

  rawsound added;
  rawsound subtracted;

  bool active;
};

#endif /* PROJECTRTPCHANNELMUX_H */
