

#ifndef PROJECTRTPCHANNELMUX_H
#define PROJECTRTPCHANNELMUX_H

#include <boost/asio.hpp>
#include <memory>

#include "projectrtpchannel.h"

class projectchannelmux:
  public std::enable_shared_from_this< projectchannelmux >
{
public:
  projectchannelmux( boost::asio::io_context &iocontext );
  ~projectchannelmux();
  typedef std::shared_ptr< projectchannelmux > pointer;
  static pointer create( boost::asio::io_context &iocontext );

  void handletick( const boost::system::error_code& error );
  void checkfordtmf( std::shared_ptr< projectrtpchannel > chan, rtppacket *src );
  void postrtpdata( std::shared_ptr< projectrtpchannel > srcchan, std::shared_ptr< projectrtpchannel > dstchan, rtppacket *src, uint32_t skipcount );
  inline size_t size() { return this->channels.size(); }
  void addchannel( std::shared_ptr< projectrtpchannel > chan );
  void go( void );

  std::list< std::shared_ptr< projectrtpchannel > > channels;

private:

  void checkfornewmixes( void );
  void mix2( void );
  void mixall( void );

  boost::asio::io_context &iocontext;
  boost::asio::steady_timer tick;

  boost::lockfree::stack< std::shared_ptr< projectrtpchannel > > newchannels;

  rawsound added;
  rawsound subtracted;
  int failcount;
};

typedef std::atomic_shared_ptr< projectchannelmux > atomicmuxptr;

#endif /*  */
